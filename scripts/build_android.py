"""Build an Android development APK, including Windows without symlink privileges."""
from pathlib import Path
import argparse,hashlib,json,os,shutil,struct,subprocess,sys,zipfile,time,uuid
from datetime import datetime,timezone

root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--target',choices=['x86_64','aarch64'],default='x86_64')
parser.add_argument('--version-code',type=int,default=int(time.time()))
parser.add_argument('--version-name')
parser.add_argument('--application-id')
parser.add_argument('--app-name')
options=parser.parse_args()
if not 1<=options.version_code<=2147483647:parser.error('version-code must be a positive Android signed 32-bit integer')
version_name=options.version_name or json.loads((root/'apps/desktop/src-tauri/tauri.conf.json').read_text(encoding='utf-8'))['version']+'-dev.'+datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')
target=options.target
arch,abi,triple,machine=('arm64','arm64-v8a','aarch64-linux-android',183) if target=='aarch64' else ('x86_64','x86_64','x86_64-linux-android',62)
work=Path(os.environ.get('RMUSIC_ANDROID_BUILD_DIR',str(root/'target/android-development'))).resolve()
work.mkdir(parents=True,exist_ok=True)
cli=root/'apps/desktop/node_modules/@tauri-apps/cli/tauri.js'
sdk=Path(os.environ.get('ANDROID_HOME') or os.environ.get('ANDROID_SDK_ROOT') or '')
if not (sdk/'platform-tools').is_dir():raise SystemExit('Set ANDROID_HOME to an installed Android SDK')
env={**os.environ,'CARGO_INCREMENTAL':'0','CARGO_BUILD_JOBS':'2','CARGO_PROFILE_DEV_DEBUG':'0','CARGO_TARGET_DIR':str(work/'cargo-target'),'RMUSIC_ANDROID_VERSION_CODE':str(options.version_code),'RMUSIC_ANDROID_VERSION_NAME':version_name}

if options.application_id:env['RMUSIC_ANDROID_APPLICATION_ID']=options.application_id
if options.app_name:env['RMUSIC_ANDROID_APP_NAME']=options.app_name

def run(args,cwd,name):
    with (work/(name+'.log')).open('wb') as log:
        code=subprocess.run(args,cwd=cwd,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
    if code:
        text=(work/(name+'.log')).read_text(encoding='utf-8',errors='replace')
        print('Native library built; using a verified copy because Windows denied a symlink.' if name=='android-build' and 'Failed to create a symbolic link' in text else text[-3000:])
    return code

npm='npm.cmd' if os.name=='nt' else 'npm'
if not cli.is_file() and run([npm,'ci','--no-audit','--no-fund'],root/'apps/desktop','cli-install'):raise SystemExit(1)
web=root/'apps/web'
if not (web/'node_modules/typescript/bin/tsc').is_file():raise SystemExit('Run npm ci --prefix apps/web first')
for args,name in [(['node',str(web/'node_modules/typescript/bin/tsc'),'-b','--pretty','false'],'types'),(['node',str(web/'node_modules/vite/bin/vite.js'),'build','--outDir',str(work/'web'),'--emptyOutDir'],'web')]:
    if run(args,web,name):raise SystemExit(1)
config=work/'tauri-build.json'
build_config={'build':{'frontendDist':os.path.relpath(work/'web',root/'apps/desktop/src-tauri').replace('\\','/')}}
if options.app_name:build_config['productName']=options.app_name
config.write_text(json.dumps(build_config),encoding='utf-8')
args=['node',str(cli),'android','build','--debug','--apk','--ci','--target',target,'--features','tauri-app,adaptive-sync','--config',str(config)]
code=run(args,root/'apps/desktop','android-build')
if code:
    text=(work/'android-build.log').read_text(encoding='utf-8',errors='replace')
    if os.name!='nt' or 'Failed to create a symbolic link' not in text:raise SystemExit(code)
    library=work/'cargo-target'/triple/'debug/librmusic_desktop.so'
    if not library.is_file():raise SystemExit('The native library was not built; refusing to package an older library')
    project=root/'apps/desktop/src-tauri/gen/android';destination=project/'app/src/main/jniLibs'/abi/'librmusic_desktop.so'
    destination.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(library,destination)
    if hashlib.sha256(destination.read_bytes()).digest()!=hashlib.sha256(library.read_bytes()).digest():raise SystemExit('Native library copy verification failed')
    flavor=arch[0].upper()+arch[1:]
    if run(['cmd.exe','/c',str(project/'gradlew.bat'),f':app:assemble{flavor}Debug','-x',f':app:rustBuild{flavor}Debug','--no-daemon','--max-workers=2','--console=plain'],project,'gradle-build'):raise SystemExit(1)
apk=root/'apps/desktop/src-tauri/gen/android/app/build/outputs/apk'/arch/'debug'/f'app-{arch}-debug.apk'
if not apk.is_file():
    universal=root/'apps/desktop/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk'
    if not universal.is_file():raise SystemExit('APK missing after build')
    # Current Tauri/AGP names a single selected ABI as universal; validate its ABI below.
    apk=universal
library=work/'cargo-target'/triple/'debug/librmusic_desktop.so'
# Distribution APKs retain exported JNI symbols without local symbol tables.
strip_tools=sorted((sdk/'ndk').glob('*/toolchains/llvm/prebuilt/*/bin/llvm-strip'+('.exe' if os.name=='nt' else '')))
if not strip_tools:raise SystemExit('Android NDK llvm-strip is missing')
before_strip=library.stat().st_size
if run([str(strip_tools[-1]),'--strip-unneeded',str(library)],root,'native-strip'):raise SystemExit(1)
native_changed=library.stat().st_size!=before_strip
if native_changed and code:shutil.copyfile(library,destination)
with zipfile.ZipFile(apk) as archive:
    payload=sum(entry.compress_size for entry in archive.infolist())
if native_changed or apk.stat().st_size>max(payload*1.2,payload+16*1024*1024):
    # Package the stripped library and avoid unused space in incremental ZIPs.
    apk.resolve().relative_to(root.resolve())
    backup=work/f'apk-incremental-{arch}-{options.version_code}-{uuid.uuid4().hex[:12]}.apk'
    backup.resolve().relative_to(work.resolve())
    if backup.exists():raise SystemExit('Incremental APK backup already exists; refusing to overwrite')
    shutil.move(str(apk),str(backup))
    project=root/'apps/desktop/src-tauri/gen/android';flavor='Universal' if apk.name=='app-universal-debug.apk' else arch[0].upper()+arch[1:]
    wrapper=['cmd.exe','/c',str(project/'gradlew.bat')] if os.name=='nt' else ['bash',str(project/'gradlew')]
    if run([*wrapper,f':app:assemble{flavor}Debug','-x',f':app:rustBuild{flavor}Debug','--no-daemon','--max-workers=2','--console=plain'],project,'gradle-repack'):raise SystemExit(1)
native=library.read_bytes()
if native[:6]!=b'\x7fELF\x02\x01' or struct.unpack_from('<H',native,18)[0]!=machine:raise SystemExit('Native ELF architecture disagrees with target')
alignments=[]
if target=='aarch64':
    offset=struct.unpack_from('<Q',native,32)[0];size,count=struct.unpack_from('<HH',native,54)
    for index in range(count):
        header=struct.unpack_from('<IIQQQQQQ',native,offset+index*size)
        if header[0]==1:
            alignments.append(header[7])
            if header[7]<16384 or (header[2]-header[3])%16384:raise SystemExit('ARM64 ELF LOAD segment is not 16 KB aligned')
    if not alignments:raise SystemExit('Native ELF has no LOAD segments')
with zipfile.ZipFile(apk) as archive:
    abis={name.split('/')[1] for name in archive.namelist() if name.startswith('lib/') and name.endswith('.so')}
    if abis!={abi} or archive.read(f'lib/{abi}/librmusic_desktop.so')!=native:raise SystemExit('APK native library/ABI verification failed')
result={'apk':str(apk),'bytes':apk.stat().st_size,'sha256':hashlib.sha256(apk.read_bytes()).hexdigest(),'target':target,'rust_target':triple,'abi':abi,'native_library_sha256':hashlib.sha256(native).hexdigest(),'elf_load_alignments':alignments,'version_code':options.version_code,'version_name':version_name,'debug':True,'native_audio_backend':'android_audiotrack','hardware_verified':False,'application_id':env.get('RMUSIC_ANDROID_APPLICATION_ID','com.rmusic.android.development')}
(work/'apk.json').write_text(json.dumps(result,indent=2),encoding='utf-8');print(json.dumps(result))
