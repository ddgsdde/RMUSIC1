#!/usr/bin/env python3
"""Seed only the dedicated Android QA Core. Never accepts a production data directory."""
from pathlib import Path
import json, struct, subprocess, urllib.request

ROOT = Path(__file__).resolve().parents[1] / 'artifacts/qa-core'
BASE = 'http://127.0.0.1:18874/api/v1/'
MEDIA = ROOT / 'fixtures'
MEDIA.mkdir(parents=True, exist_ok=True)

def request(path, body=None, token=None, method=None):
    req = urllib.request.Request(BASE + path, data=json.dumps(body).encode() if body is not None else None,
        headers={'Content-Type': 'application/json', **({'Authorization': 'Bearer '+token} if token else {})}, method=method)
    with urllib.request.urlopen(req, timeout=30) as response:
        return json.load(response)

status = request('status')
identity = ROOT / 'core-id'
if identity.exists() and identity.read_text() != status['core_id']:
    raise SystemExit('Refusing to seed an unexpected Core')
if not status['initialized']:
    request('setup', {'bootstrap_token': (ROOT/'data/bootstrap.token').read_text().strip(),
        'username': 'android-qa', 'password': 'RMusicAndroidQA-20261030'})
identity.write_text(status['core_id'])
token = request('auth/login', {'username': 'android-qa', 'password': 'RMusicAndroidQA-20261030'})['token']
(ROOT/'session.json').write_text(json.dumps({'token': token}))
(ROOT/'session.json').chmod(0o600)
for i, rate in enumerate([96000, 44100, 48000]):
    target = MEDIA / f'track-{i+1}.flac'
    if not target.exists():
        subprocess.run(['ffmpeg','-v','error','-threads','1','-f','lavfi','-i',
            f'sine=frequency={220*(i+1)}:sample_rate={rate}:duration=18', '-ac','2','-c:a','flac','-sample_fmt','s32',
            '-metadata',f'title=Android QA {i+1}','-metadata','artist=R-Music Test','-metadata','album=Native Journey',
            '-metadata',f'track={i+1}',str(target)], check=True)
    target.with_suffix('.lrc').write_text('[00:00.00]Native Journey\n[00:03.00]Follow the music\n[00:06.00]A quiet moment\n[00:09.00]Across the city\n[00:12.00]On the road\n[00:15.00]Music stays with you\n')
cover = MEDIA/'cover.jpg'
if not cover.exists():
    subprocess.run(['ffmpeg','-v','error','-threads','1','-f','lavfi','-i','testsrc2=size=512x512','-frames:v','1',str(cover)],check=True)
# One second of generated DSD64, two block-interleaved channels. Never a DAC/DoP test.
dsf = MEDIA/'DSD64-QA.dsf'
if not dsf.exists():
    count, block, channels, rate = 2822400, 4096, 2, 2822400
    blocks = (count//8+block-1)//block
    data = bytes([0x69]) * (blocks*block*channels)
    header = b'DSD '+struct.pack('<QQQ',28,28+52+12+len(data),0)
    fmt = b'fmt '+struct.pack('<QIIIIIIQII',52,1,0,2,channels,rate,1,count,block,0)
    dsf.write_bytes(header+fmt+b'data'+struct.pack('<Q',len(data)+12)+data)
request('library/roots', {'path': str(MEDIA)}, token)
request('library/scan', {}, token)
request('settings', {'mobile_opus_enabled': True, 'cache_limit_bytes': 67108864}, token, 'PATCH')
print('Seeded independent QA Core: 3 PCM/FLAC tracks + 1 generated DSD64; no user data.')
