"""Build matching Windows programs and verify the UI actually embedded in the client."""
from pathlib import Path
import argparse,hashlib,json,os,shutil,subprocess,uuid

root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--web-dir',type=Path,help='Reuse an already-built UI directory')
parser.add_argument('--asio-sdk',type=Path,default=os.environ.get('CPAL_ASIO_DIR'))
parser.add_argument('--libclang',type=Path,default=os.environ.get('LIBCLANG_PATH'))
parser.add_argument('--wasapi-only',action='store_true')
options=parser.parse_args()
work=Path(os.environ.get('RMUSIC_WINDOWS_BUILD_DIR',root/'target/windows-development')).resolve()
work.mkdir(parents=True,exist_ok=True)
if not options.web_dir:
    subprocess.run(['npm.cmd','--prefix',str(root/'apps/web'),'run','build'],cwd=root,check=True)
web=(options.web_dir or root/'apps/web/dist').resolve()
if not (web/'index.html').is_file():parser.error('A built UI with index.html is required')
frozen=work/('web-'+uuid.uuid4().hex[:12]);shutil.copytree(web,frozen)
# FrontendDist interprets C:\... as a URL. A relative path is required on Windows.
relative=os.path.relpath(frozen,root/'apps/desktop/src-tauri').replace('\\','/')
env={**os.environ,'CARGO_INCREMENTAL':'0','CARGO_BUILD_JOBS':'2','CARGO_PROFILE_DEV_DEBUG':'0',
     'TAURI_CONFIG':json.dumps({'build':{'frontendDist':relative,'devUrl':None}})}
features=['rmusic-desktop/tauri-app','rmusic-desktop/adaptive-sync','rmusic-bridge/adaptive-sync']
if not options.wasapi_only:
    if not options.asio_sdk or not (Path(options.asio_sdk)/'common/asio.h').is_file():parser.error('Set --asio-sdk or explicitly select --wasapi-only')
    if not options.libclang or not (Path(options.libclang)/'libclang.dll').is_file():parser.error('Set --libclang to the directory containing libclang.dll')
    env.update(CPAL_ASIO_DIR=str(Path(options.asio_sdk).resolve()),LIBCLANG_PATH=str(Path(options.libclang).resolve()))
    features+=['rmusic-desktop/asio-output','rmusic-bridge/asio-output']
subprocess.run(['cargo','build','-p','rmusic-core','-p','rmusic-desktop','-p','rmusic-bridge','--features',','.join(features)],cwd=root,env=env,check=True)
client=root/'target/debug/rmusic-desktop.exe'
manifest=json.loads(subprocess.check_output([str(client),'--ui-build-info'],cwd=root,encoding='utf-8'))
actual={item['path'].lstrip('/'):item for item in manifest['assets']}
if not actual or 'index.html' not in actual:raise SystemExit('Client has no embedded UI; refusing delivery')
for file in frozen.rglob('*'):
    if not file.is_file() or file.name=='index.html':continue
    name=file.relative_to(frozen).as_posix()
    if name not in actual or hashlib.file_digest(file.open('rb'),'sha256').hexdigest()!=actual[name]['sha256']:
        raise SystemExit('Embedded UI differs from build input: '+name)
manifest.update(client=str(client),client_sha256=hashlib.file_digest(client.open('rb'),'sha256').hexdigest(),frontend=str(frozen))
(work/'windows-ui.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps({'client':str(client),'embedded_assets':len(actual),'client_sha256':manifest['client_sha256'],'evidence':str(work/'windows-ui.json')}))
