#!/usr/bin/env python3
"""Isolated synthetic Core for native/WebView visual parity and real HAL acceptance.
No user media, production credentials, discovery or physical output is used.
"""
import json, os, pathlib, secrets, signal, socket, subprocess, time, urllib.request
from verify_client import ROOT, wait
from verify_features import png

work = ROOT/'artifacts'/time.strftime('stage-round4-%Y%m%dT%H%M%S')
(work/'empty').mkdir(parents=True)
with socket.socket() as sock:
    sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
base=f'http://127.0.0.1:{port}'
env=dict(os.environ,RMUSIC_DATA_DIR=str(work/'core'),RMUSIC_CACHE_DIR=str(work/'cache'),RMUSIC_PLUGIN_DIR=str(work/'empty'),RMUSIC_BUILTIN_PLUGIN_DIR=str(work/'empty'),RMUSIC_WEB_DIR=str(ROOT.parent/'apps/web/dist'),RMUSIC_BIND='127.0.0.1',RMUSIC_PORT=str(port),RMUSIC_DISCOVERY='0',RMUSIC_FFMPEG='/opt/homebrew/bin/ffmpeg',RMUSIC_FFPROBE='/opt/homebrew/bin/ffprobe')
log=(work/'core.log').open('w')
core=subprocess.Popen([ROOT/'target/debug/rmusic-core'],env=env,stdout=log,stderr=log)
token=None

def api(route,body=None,method=None):
    req=urllib.request.Request(base+'/api/v1'+route,data=None if body is None else json.dumps(body).encode(),headers={'Content-Type':'application/json'}|({'Authorization':'Bearer '+token} if token else {}),method=method)
    with urllib.request.urlopen(req,timeout=20) as response:
        data=response.read();return json.loads(data) if data else None

def health():
    try:return api('/status')
    except OSError:return None

def stop(*_):raise SystemExit(0)
signal.signal(signal.SIGTERM,stop);signal.signal(signal.SIGINT,stop)
try:
    status=wait(health)
    password='qa-'+secrets.token_hex(16)
    token=api('/setup',dict(bootstrap_token=(work/'core/bootstrap.token').read_text().strip(),username='stage-qa',password=password))['token']
    settings=api('/settings');assert settings['cache_enabled'] is True
    music=work/'music';music.mkdir()
    for i,(title,duration) in enumerate([('脉冲与星河',1200),('曲尾交接',18),('重拾旋律',18)]):
        folder=music/str(i);folder.mkdir();png(folder/'cover.png',i)
        expression='0.045*sin(2*PI*55*t)*exp(-18*mod(t,0.5))+0.012*sin(2*PI*440*t)*(0.3+0.7*sin(PI*t)^2)+0.008*sin(2*PI*4000*t)*exp(-30*mod(t+0.25,0.5))'
        subprocess.run(['/opt/homebrew/bin/ffmpeg','-v','error','-f','lavfi','-i',f"aevalsrc='{expression}':s=48000:d={duration}",'-ac','2','-c:a','flac','-sample_fmt','s32','-metadata','title='+title,'-metadata','artist=Stage QA','-metadata','album=Native parity '+str(i),str(folder/(title+'.flac'))],check=True)
        lines=[]
        for n in range(duration//4):
            at=f'{n*4//60:02}:{n*4%60:02}.00'
            text=['Follow the rhythm','长句也应平顺滚动，星河在此刻流淌','Breathe with the music','光与声音，都在此刻相逢'][n%4]
            lines.append(f'[{at}]{text}')
            if n%4 in [0,2]:lines.append(f'[{at}]跟随节奏，听见每一处细节')
        (folder/(title+'.lrc')).write_text('\n'.join(lines))
    api('/library/roots',{'path':str(music)});job=api('/library/scan',{})['id']
    wait(lambda:api('/jobs?id='+job+'&limit=1')['items'][0]['status']=='completed')
    tracks=api('/library/tracks')['items'];assert len(tracks)==3
    api('/playlists',{'name':'舞台对照与曲尾验收','track_ids':[t['id'] for t in tracks]})
    connection=dict(address=base,username='stage-qa',password=password,token=token,core_id=status['core_id'],tracks=tracks)
    target=work/'connection.json';target.write_text(json.dumps(connection));target.chmod(0o600)
    (ROOT/'artifacts/latest-stage-round4.txt').write_text(str(work))
    print('STAGE_QA_READY '+str(work),flush=True)
    while True:time.sleep(1)
finally:
    if core.poll() is None:core.terminate();core.wait(timeout=30)
    log.close()
