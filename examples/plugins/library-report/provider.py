#!/usr/bin/env python3
"""Explicit read-only example; no external requests, credentials or file-system scans."""
import json, pathlib, sys
MANIFEST=json.loads(pathlib.Path(__file__).with_name('manifest.json').read_text())
def dispatch(method,params):
    if method=='manifest': return MANIFEST
    if method=='health': return {'status':'ready'}
    if method=='extension_invoke' and params.get('version')==1 and params.get('method')=='summarize':
        context=params['context']
        return {'version':1,'output':{'tracks':context['track_count'],'sources':len(context['source_counts'])}}
    raise ValueError('unsupported_method')
for raw in sys.stdin:
    try:
        request=json.loads(raw)
        reply={'id':request['id'],'result':dispatch(request['method'],request.get('params',{}))}
    except Exception:
        reply={'id':request.get('id') if isinstance(locals().get('request'),dict) else None,'error':{'code':'unsupported_request','message':'请求不符合扩展示例合同'}}
    print(json.dumps(reply,ensure_ascii=False),flush=True)
