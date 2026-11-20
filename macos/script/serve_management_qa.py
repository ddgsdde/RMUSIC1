#!/usr/bin/env python3
"""An empty, isolated cache-disabled Core for native management UI acceptance.
No audio output, provider credentials or production library is touched.
"""
import argparse, json, os, pathlib, socket, subprocess, time, urllib.request
parser = argparse.ArgumentParser()
parser.add_argument('--workspace', action='store_true')
parser.add_argument('--related', action='store_true')
parser.add_argument('--music', action='store_true')
args = parser.parse_args()
if args.workspace and args.related: parser.error("workspace and related fixtures use different synthetic plugins; select one")
root = pathlib.Path(__file__).resolve().parents[1]
work = root / 'artifacts' / time.strftime('management-ui-%Y%m%dT%H%M%S')
work.mkdir(parents=True)
(work / 'empty-plugins').mkdir()
with socket.socket() as listener:
    listener.bind(('127.0.0.1', 0))
    port = listener.getsockname()[1]
base = f'http://127.0.0.1:{port}'
env = os.environ | {
    'RMUSIC_DATA_DIR': str(work / 'data'), 'RMUSIC_CACHE_DIR': str(work / 'cache'),
    'RMUSIC_PLUGIN_DIR': str(work / 'empty-plugins'), 'RMUSIC_BUILTIN_PLUGIN_DIR': str(work / 'empty-plugins'),
    'RMUSIC_WEB_DIR': str(root.parent / 'apps/web/dist'), 'RMUSIC_CACHE_ENABLED': '0',
    'RMUSIC_BIND': '127.0.0.1', 'RMUSIC_PORT': str(port), 'RMUSIC_DISCOVERY': '0',
    'RMUSIC_FFMPEG': '/opt/homebrew/bin/ffmpeg', 'RMUSIC_FFPROBE': '/opt/homebrew/bin/ffprobe',
}
username, password = 'native-management-qa', 'synthetic-local-ui-only-password'
def api(path, token=None, body=None):
    request = urllib.request.Request(base + '/api/v1' + path,
        data=json.dumps(body).encode() if body is not None else None,
        headers={'Content-Type': 'application/json'} | ({'Authorization': 'Bearer ' + token} if token else {}))
    with urllib.request.urlopen(request, timeout=5) as response:
        return json.load(response)
with (work / 'core.log').open('w') as log:
    child = subprocess.Popen([root / 'target/debug/rmusic-core'], env=env, stdout=log, stderr=log)
    try:
        for _ in range(100):
            try:
                status = api('/status')
                break
            except (OSError, ValueError):
                if child.poll() is not None: raise RuntimeError('QA Core exited')
                time.sleep(.1)
        else: raise RuntimeError('QA Core startup timed out')
        token = api('/setup', body={'bootstrap_token': (work / 'data/bootstrap.token').read_text().strip(), 'username': username, 'password': password})['token']
        settings = api('/settings', token)
        assert settings['cache_enabled'] is False and settings['cache_forced_off'] is False
        if args.music:
            from app_audit_fixture import seed_music
            seed_music(work, api, token)
        if args.related:
            from related_catalog_fixture import seed
            seed(work,api,token)
        if args.workspace:
            from workspace_qa_fixture import seed
            seed(work, api, token)
        report = {'base': base, 'work': str(work), 'cache_enabled': False, 'core_id': status['core_id'], 'username': username, 'password': password}
        (root / 'artifacts/management-ui-latest.json').write_text(json.dumps(report))
        (root / 'artifacts/management-ui-latest.json').chmod(0o600)
        print(json.dumps({'base': base, 'work': str(work), 'cache_enabled': False}), flush=True)
        child.wait()
    except KeyboardInterrupt:
        pass
    finally:
        if child.poll() is None:
            child.terminate()
            try: child.wait(timeout=10)
            except subprocess.TimeoutExpired: child.kill(); child.wait()
