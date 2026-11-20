#!/bin/sh
set -eu
cd "$(dirname "$0")"
out=../../apps/web/src/secure/engine
notices=../../apps/web/public/licenses
mkdir -p "$out" "$notices"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
GOOS=js GOARCH=wasm CGO_ENABLED=0 go build -trimpath -buildvcs=false -ldflags='-s -w -buildid=' -o "$tmp/tls.wasm" .
gzip -9 -n -c "$tmp/tls.wasm" > "$out/core-tls.wasm.gz"
install -m 0644 "$(go env GOROOT)/lib/wasm/wasm_exec.js" "$out/wasm_exec.js"
license="$(go env GOROOT)/LICENSE"
[ -f "$license" ] || license="$(go env GOROOT)/../LICENSE"
install -m 0644 "$license" "$notices/browser-tls-Go.txt"
install -m 0644 "$(go list -m -f '{{.Dir}}' github.com/coder/websocket)/LICENSE.txt" "$notices/browser-tls-WebSocket.txt"

