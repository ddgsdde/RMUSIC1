"""A partial local album plus an explicit synthetic platform; never opens audio hardware."""
import json,subprocess,time

def seed(work,api,token):
    music=work/'music';music.mkdir()
    subprocess.run(['/opt/homebrew/bin/ffmpeg','-v','error','-f','lavfi','-i','anullsrc=r=44100:cl=stereo','-t','4','-metadata','title=夜航','-metadata','artist=林间','-metadata','album=海岸线','-metadata','track=1',str(music/'夜航.wav')],check=True)
    directory=work/'empty-plugins/example';directory.mkdir()
    (directory/'manifest.json').write_text(json.dumps({'id':'example','name':'验收平台（合成）','version':'0.1','runtime':'python>=3.12','entrypoint':'provider.py','capabilities':['metadata','related_catalog_v1']}))
    (directory/'provider.py').write_text('''import json,sys
for line in sys.stdin:
 r=json.loads(line);p=r.get('params',{});method=r['method'];v={}
 if method=='health':v={'status':'ready','authenticated':False}
 elif method=='metadata':v={'matched':False,'source':'example'}
 elif method=='related_catalog':
  local=p['track'];page=p.get('page',1);kind=p.get('kind','album')
  anchor=local|dict(id='example:Anchor001',source='example',recording_id='example:Anchor001',provider_data={'album_mid':'Album001','artist_mids':['Singer001']})
  second=anchor|dict(id='example:Song002',recording_id='example:Song002',title='远岸',track_number=2)
  third=anchor|dict(id='example:Song003',recording_id='example:Song003',title='夜航 (Live)',track_number=3,provider_data=anchor['provider_data']|{'subtitle':'Live'})
  items=[anchor,second] if page==1 else [third]
  v=dict(matched=True,source='example',kind=kind,page=page,anchor=anchor,artists=[dict(mid='Singer001',name=local['artist'])],collection=dict(id='Album001' if kind=='album' else 'Singer001',name=local['album'] if kind=='album' else local['artist']),identity_basis='exact_tags_duration_unique_candidate',items=items,total=3,next_page=2 if page==1 else None)
 print(json.dumps(dict(id=r['id'],result=v),ensure_ascii=False),flush=True)
''')
    api('/library/roots',token,{'path':str(music)})
    job=api('/library/scan',token,{})['id']
    for _ in range(100):
        rows=api('/jobs?id='+job+'&limit=1',token)['items']
        if rows and rows[0]['status']=='completed':break
        time.sleep(.1)
    else:raise AssertionError('scan timed out')
    local=api('/library/tracks?source=local',token)['items'][0]
    page=api('/library/tracks/'+local['id']+'/related',token)
    assert page['items'][0]['id']==local['id'] and page['total']==3 and page['items'][1]['source']=='example',page
    assert api('/cache',token)['used_bytes']==0
    assert api('/library/tracks?source=example',token)['total']==0
    assert api('/library/albums',token)['total']==1
    (work/'catalog-fixture.json').write_text(json.dumps({'local':local,'page':page},ensure_ascii=False,indent=2))
