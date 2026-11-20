#!/usr/bin/env python3
"""Loopback-only isolated QA proxy. Logs transfer offsets, never headers/tokens/login bodies."""
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from pathlib import Path
import urllib.request, urllib.error, time, json, threading
ROOT=Path(__file__).resolve().parents[1]/'artifacts'
LOG=ROOT/'qa-proxy-transfers.jsonl'
LOCK=threading.Lock()
class Handler(BaseHTTPRequestHandler):
    protocol_version='HTTP/1.1'
    def log_message(self,*args): pass
    def process(self):
        size=int(self.headers.get('Content-Length','0'))
        if size>8*1024*1024:self.send_error(413);return
        body=self.rfile.read(size) if size else None
        headers={k:v for k,v in self.headers.items() if k.lower() in ['authorization','content-type','range','if-range']}
        request=urllib.request.Request('http://127.0.0.1:18874'+self.path,data=body,headers=headers,method=self.command)
        try:
            try: upstream=urllib.request.urlopen(request,timeout=15)
            except urllib.error.HTTPError as e:upstream=e
            with upstream:
                self.send_response(upstream.status)
                for k,v in upstream.headers.items():
                    if k.lower() not in ['connection','transfer-encoding','server','date']:self.send_header(k,v)
                self.send_header('Connection','close');self.end_headers()
                transferred=0
                try:
                    if self.command!='HEAD':
                        while block:=upstream.read(16384):
                            self.wfile.write(block);self.wfile.flush();transferred+=len(block)
                            if '/downloads/' in self.path and '/file' in self.path:time.sleep(.0625)
                except (BrokenPipeError,ConnectionResetError):pass
                finally:
                    if '/downloads/' in self.path and '/file' in self.path:
                        with LOCK, LOG.open('a') as out:out.write(json.dumps({'method':self.command,'range':self.headers.get('Range'),'status':upstream.status,'bytes':transferred})+'\n')
        except Exception:
            self.close_connection=True
    do_GET=do_POST=do_PUT=do_PATCH=do_DELETE=do_HEAD=process
ROOT.mkdir(parents=True,exist_ok=True)
print('QA file proxy on 127.0.0.1:18875 -> isolated Core18874',flush=True)
ThreadingHTTPServer(('127.0.0.1',18875),Handler).serve_forever()
