//go:build js && wasm

// Encrypted browser requests use the same TLS-over-WebSocket listener as native clients.
// net/http provides framing, streaming, connection pooling and cancellation.
package main

import (
	"context"
	"crypto/ed25519"
	"crypto/subtle"
	"crypto/tls"
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"sync"
	"syscall/js"
	"time"

	"github.com/coder/websocket"
)

type transfer struct {
	cancel context.CancelFunc
	body   io.ReadCloser
}

var transfers = map[int]*transfer{}
var mu sync.Mutex
var transport *http.Transport
var binding string

func client(endpoint, host, key string) (*http.Transport, error) {
	raw, err := base64.RawURLEncoding.DecodeString(key)
	if err != nil || len(raw) != ed25519.PublicKeySize {
		return nil, errors.New("Core 公钥无效")
	}
	identity := endpoint + " " + host + " " + key
	mu.Lock()
	defer mu.Unlock()
	if binding == identity {
		return transport, nil
	}
	if transport != nil {
		transport.CloseIdleConnections()
	}
	cfg := &tls.Config{ServerName: host, MinVersion: tls.VersionTLS13, InsecureSkipVerify: true, // Core uses a self-signed identity certificate; check its pinned signing key and SAN.
		VerifyConnection: func(s tls.ConnectionState) error {
			if len(s.PeerCertificates) == 0 {
				return errors.New("缺少 Core 身份证书")
			}
			cert := s.PeerCertificates[0]
			pub, ok := cert.PublicKey.(ed25519.PublicKey)
			now := time.Now()
			if !ok || subtle.ConstantTimeCompare(pub, raw) != 1 || cert.VerifyHostname(host) != nil || now.Before(cert.NotBefore) || now.After(cert.NotAfter) {
				return errors.New("Core 身份验证失败")
			}
			return nil
		}}
	transport = &http.Transport{MaxIdleConnsPerHost: 6, MaxConnsPerHost: 12, IdleConnTimeout: 30 * time.Second, MaxResponseHeaderBytes: 64 << 10, ResponseHeaderTimeout: 5 * time.Minute,
		DialTLSContext: func(ctx context.Context, _, addr string) (net.Conn, error) {
			if addr != host+":443" {
				return nil, errors.New("无效的 Core 目的地")
			}
			dial, cancel := context.WithTimeout(ctx, 18*time.Second)
			defer cancel()
			socket, _, err := websocket.Dial(dial, endpoint, nil)
			if err != nil {
				return nil, err
			}
			conn := tls.Client(websocket.NetConn(context.Background(), socket, websocket.MessageBinary), cfg)
			if err := conn.HandshakeContext(dial); err != nil {
				socket.CloseNow()
				return nil, err
			}
			return conn, nil
		}}
	binding = identity
	return transport, nil
}

type promiseResult struct {
	value js.Value
	err   error
}

func await(p js.Value) (js.Value, error) {
	out := make(chan promiseResult, 1)
	yes := js.FuncOf(func(_ js.Value, args []js.Value) any { out <- promiseResult{value: args[0]}; return nil })
	no := js.FuncOf(func(_ js.Value, _ []js.Value) any {
		out <- promiseResult{err: errors.New("浏览器读取已中断")}
		return nil
	})
	p.Call("then", yes, no)
	r := <-out
	yes.Release()
	no.Release()
	return r.value, r.err
}

type upload struct {
	reader js.Value
	buf    []byte
}

func (u *upload) Read(p []byte) (int, error) {
	if len(u.buf) == 0 {
		x, err := await(u.reader.Call("read"))
		if err != nil {
			return 0, err
		}
		if x.Get("done").Bool() {
			return 0, io.EOF
		}
		v := x.Get("value")
		u.buf = make([]byte, v.Get("byteLength").Int())
		js.CopyBytesToGo(u.buf, v)
	}
	n := copy(p, u.buf)
	u.buf = u.buf[n:]
	return n, nil
}
func (u *upload) Close() error {
	u.reader.Call("cancel").Call("catch", js.Global().Get("Boolean"))
	return nil
}

func async(f func() (any, error)) any {
	executor := js.FuncOf(func(_ js.Value, a []js.Value) any {
		resolve, reject := a[0], a[1]
		go func() {
			value, err := f()
			if err != nil {
				reject.Invoke(js.Global().Get("Error").New(err.Error()))
			} else {
				resolve.Invoke(value)
			}
		}()
		return nil
	})
	p := js.Global().Get("Promise").New(executor)
	executor.Release()
	return p
}
func finish(id int) {
	mu.Lock()
	t := transfers[id]
	delete(transfers, id)
	mu.Unlock()
	if t != nil {
		t.cancel()
		if t.body != nil {
			t.body.Close()
		}
	}
}
func begin(_ js.Value, a []js.Value) any {
	id := a[0].Int()
	ctx, cancel := context.WithCancel(context.Background())
	t := &transfer{cancel: cancel}
	mu.Lock()
	transfers[id] = t
	mu.Unlock()
	return async(func() (ret any, err error) {
		defer func() {
			if err != nil {
				finish(id)
			}
		}()
		host := a[2].String()
		tr, err := client(a[1].String(), host, a[3].String())
		if err != nil {
			return nil, err
		}
		path := a[5].String()
		if !strings.HasPrefix(path, "/api/v1/") {
			return nil, errors.New("无效的 Core API 路径")
		}
		var body io.Reader
		if !a[7].IsNull() {
			body = &upload{reader: a[7]}
		}
		req, err := http.NewRequestWithContext(ctx, a[4].String(), "https://"+host+path, body)
		if err != nil {
			return nil, err
		}
		for i := 0; i < a[6].Length(); i++ {
			pair := a[6].Index(i)
			req.Header.Add(pair.Index(0).String(), pair.Index(1).String())
		}
		resp, err := tr.RoundTrip(req)
		if err != nil {
			return nil, fmt.Errorf("Core 加密连接失败：%w", err)
		}
		mu.Lock()
		if transfers[id] != t {
			mu.Unlock()
			resp.Body.Close()
			return nil, context.Canceled
		}
		t.body = resp.Body
		mu.Unlock()
		headers := []any{}
		for k, values := range resp.Header {
			for _, v := range values {
				headers = append(headers, []any{k, v})
			}
		}
		return map[string]any{"status": resp.StatusCode, "headers": headers}, nil
	})
}
func read(_ js.Value, a []js.Value) any {
	id := a[0].Int()
	return async(func() (any, error) {
		mu.Lock()
		t := transfers[id]
		mu.Unlock()
		if t == nil {
			return nil, context.Canceled
		}
		b := make([]byte, 48<<10)
		n, err := t.body.Read(b)
		if n > 0 {
			v := js.Global().Get("Uint8Array").New(n)
			js.CopyBytesToJS(v, b[:n])
			return v, nil
		}
		if err != nil {
			finish(id)
			if err == io.EOF {
				return nil, nil
			}
			return nil, err
		}
		return js.Global().Get("Uint8Array").New(0), nil
	})
}
func main() {
	api := js.Global().Get("Object").New()
	api.Set("begin", js.FuncOf(begin))
	api.Set("read", js.FuncOf(read))
	api.Set("close", js.FuncOf(func(_ js.Value, a []js.Value) any { go finish(a[0].Int()); return nil }))
	js.Global().Set("rmusicTLS", api)
	select {}
}
