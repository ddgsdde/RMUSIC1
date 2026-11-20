"""Synthetic-only provider, device and statistics for an isolated management Core."""
import datetime, json, pathlib, sqlite3, time


def seed(work, api, token):
    plugin = work / 'empty-plugins/example'
    plugin.mkdir()
    manifest = {'id': 'example', 'name': '验收音源（合成）', 'version': '0.1.0', 'protocol_version': 1,
                'api_version': '1', 'core_api': '>=0.1.0,<0.2.0',
                'runtime': {'kind': 'python', 'version': '>=3.12'}, 'entrypoint': 'provider.py',
                'capabilities': ['playlists', 'account_context_v1'], 'verified': False}
    (plugin / 'manifest.json').write_text(json.dumps(manifest))
    (plugin / 'provider.py').write_text('''import json,sys
manifest=json.load(open(__file__.replace('provider.py','manifest.json')))
def track(i):
 return dict(id=f'example:track{i}',title=f'合成歌曲 {i:03}',artist='Synthetic Artist',album='合成专辑',album_artist='Synthetic Artist',duration=240,track_number=i,disc_number=1,source='example',format='flac',sample_rate=96000,bit_depth=24,channels=2,bitrate=0,cover_url=None,recording_id=f'qa{i}',available=True,created_at=0,provider_data={})
for line in sys.stdin:
 r=json.loads(line);m=r['method'];p=r.get('params',{});page=p.get('page',1)
 if m=='manifest':v=manifest
 elif m=='health':v=dict(status='ready',authenticated=True,account_context='synthetic-account')
 elif m=='playlists':
  kinds=['liked','created','collected'];collection=p.get('collection','all')
  items=[dict(id=k,name={'liked':'合成我喜欢','created':'长歌单 · 205 首','collected':'收藏歌单 · 合成'}[k],track_count=205,cover_url=None,source='example',kind=k,writable=False) for k in kinds if collection=='all' or collection==k]
  v=dict(items=items[page-1:page],next_page=page+1 if page<len(items) else None)
 elif m=='playlist_tracks':v=dict(playlist_id=p['playlist_id'],name='长歌单 · 205 首',total=205,items=[track(i) for i in range((page-1)*100+1,min(page*100,205)+1)],next_page=page+1 if page<3 else None)
 else:v={}
 print(json.dumps(dict(id=r['id'],result=v),ensure_ascii=False),flush=True)
''')
    request = lambda path, body=None: api(path, token, body)
    pages = [request(f'/providers/example/playlists?page={page}') for page in [1, 2, 3]]
    assert [p['items'][0]['kind'] for p in pages] == ['liked', 'created', 'collected']
    tracks = [request(f'/providers/example/playlists/created/tracks?page={page}') for page in [1, 2, 3]]
    assert [len(p['items']) for p in tracks] == [100, 100, 5]
    request('/playlists', {'name': '本地播放列表 · 合成', 'track_ids': [t['id'] for t in tracks[0]['items'][:6]]})
    request('/playlist-directory/refresh', {})
    for _ in range(100):
        directory = request('/playlist-directory')['items']
        if len(directory) == 3: break
        time.sleep(.2)
    else: raise AssertionError('cloud directory did not traverse all three pages')
    descriptor = dict(id='qa-remote-output', name='远程虚拟 DAC（无硬件）', kind='wasapi', configurable=True, native_device_id='fixture-only', default=False)
    request('/devices/register', {'id': 'qa-remote-host', 'name': '远程测试电脑（合成）', 'platform': 'windows', 'outputs': [descriptor]})
    # No endpoint is registered online and no audio can be dispatched to this fixture.
    with sqlite3.connect(work / 'data/library.db') as db:
        for i in range(7):
            date=(datetime.datetime.now(datetime.timezone.utc)-datetime.timedelta(days=i)).strftime('%Y-%m-%d')
            at=int(time.time())-i*86400
            db.execute('INSERT INTO daily_stats(day,track_id,title,artist,cover_url,play_count,skip_count,play_seconds,last_played_at) VALUES(?,?,?,?,?,?,?,?,?)', (date,'example:track1','合成歌曲 001','Synthetic Artist',None,3,0,2400-i*200,at))
        db.execute("INSERT OR REPLACE INTO listening_metadata VALUES('example:track1','合成专辑','Synthetic Artist','example')")
        at=int(time.time()*1000)
        db.execute('INSERT INTO play_sessions(id,zone_id,track_id,title,artist,duration,started_at,last_report_at,last_position,last_status,runtime_id,play_seconds,counted,finished_at,finish_reason) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)', ('fixture-session','fixture-zone','example:track1','合成歌曲 001','Synthetic Artist',240,at-240000,at,240,'stopped','fixture',240,1,at,'ended'))
    (work/'workspace-fixture-result.json').write_text(json.dumps({'synthetic': True,'cloud_directory_pages':3,'cloud_directory_count':len(directory),'track_page_counts':[100,100,5],'physical_output':False},indent=2))
