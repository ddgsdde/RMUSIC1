#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-run}"
PRODUCT="${2:-RMusic}"
case "$MODE" in run|--build-only|--verify|--logs|--telemetry|--debug) ;; *) echo 'Usage: build_and_run.sh [run|--build-only|--verify|--logs|--telemetry|--debug] [RMusic|RMusicBridge|RMusicNative]' >&2; exit 2;; esac
case "$PRODUCT" in RMusic|RMusicBridge|RMusicNative) ;; *) echo 'Unknown macOS product' >&2; exit 2;; esac
# A separate build-only destination allows packaging without touching a running client.
if [[ -n "${RMUSIC_MAC_DIST_DIR:-}" && "$MODE" != --build-only ]]; then echo 'A custom dist directory requires --build-only' >&2; exit 2; fi
TASK_DIST_DIR="${RMUSIC_MAC_DIST_DIR:-$ROOT_DIR/dist}"
if [[ "$TASK_DIST_DIR" != /* ]]; then echo 'Dist directory must be absolute' >&2; exit 2; fi
if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then echo 'This workspace currently targets Apple Silicon macOS 27+' >&2; exit 2; fi
if ! command -v cargo >/dev/null; then
    TASK_TOOLCHAIN_BIN="$(dirname "$(rustup which rustc)")"
    export PATH="$TASK_TOOLCHAIN_BIN:$PATH"
fi
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 MACOSX_DEPLOYMENT_TARGET=27.0
# Stop only the selected development bundle, never the user's installed client or Core.
BUNDLE_ID="com.rmusic.mac.client.development"
MENU_ONLY=false
TASK_SWIFT_ARGS=()
TASK_VERSION=0.1.0
if [[ "$PRODUCT" == RMusicNative ]]; then
    export RMUSIC_NATIVE_V2=1
    BUNDLE_ID="com.rmusic.mac.client.native-v2.development"
    TASK_VERSION=0.2.59-dev
    TASK_SWIFT_ARGS=(-c release --scratch-path "${RMUSIC_MAC_SWIFT_DIR:-$ROOT_DIR/.build/native-v2}")
fi
TASK_APP_NAME="$PRODUCT"
if [[ "${RMUSIC_MAC_PUBLIC:-0}" == 1 ]]; then
    [[ "$MODE" == --build-only && "$PRODUCT" == RMusicNative && "${RMUSIC_MAC_QA:-0}" != 1 ]] || exit 2
    BUNDLE_ID="com.rmusic.mac.client"
    TASK_APP_NAME="R-Music"
    TASK_VERSION=0.1.0-preview.1
    TASK_SOURCE="$(cd "$ROOT_DIR/.." && pwd)"
    TASK_SEPARATOR=$'\x1f'
    export CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS:+$CARGO_ENCODED_RUSTFLAGS$TASK_SEPARATOR}--remap-path-prefix=$HOME=/build-user$TASK_SEPARATOR--remap-path-prefix=$TASK_SOURCE=/src"
    export CARGO_PROFILE_DEV_DEBUG=0
    export CC_SHELL_ESCAPED_FLAGS=1
    export CFLAGS="${CFLAGS:+$CFLAGS }\"-ffile-prefix-map=$HOME=/build-user\" \"-ffile-prefix-map=$TASK_SOURCE=/src\""
fi
if [[ "${RMUSIC_MAC_QA:-0}" == 1 ]]; then
    BUNDLE_ID="com.rmusic.mac.client.qa"; TASK_APP_NAME="RMusicQA"
    if [[ -n "${RMUSIC_MAC_QA_PROFILE:-}" ]]; then
        if [[ ! "$RMUSIC_MAC_QA_PROFILE" =~ ^[a-z][a-z0-9-]{0,30}$ ]]; then echo 'Invalid QA profile' >&2; exit 2; fi
        BUNDLE_ID="$BUNDLE_ID.$RMUSIC_MAC_QA_PROFILE"; TASK_APP_NAME="$TASK_APP_NAME-$RMUSIC_MAC_QA_PROFILE"
    fi
fi
if [[ "$PRODUCT" == RMusicBridge ]]; then BUNDLE_ID="com.rmusic.mac.bridge.development"; MENU_ONLY=true; fi
# Distinct apps/QA profiles must not reuse a main Mach-O UUID (Apple TN3178).
# A role-specific read-only identity section changes the linker's content hash.
TASK_IDENTITY_FILE="$ROOT_DIR/.build/bundle-identities/$BUNDLE_ID.txt"
mkdir -p "$(dirname "$TASK_IDENTITY_FILE")"
printf '%s\n' "$BUNDLE_ID" > "$TASK_IDENTITY_FILE"
TASK_SWIFT_ARGS+=(-Xlinker -sectcreate -Xlinker __TEXT -Xlinker __rmusic_id -Xlinker "$TASK_IDENTITY_FILE")
if [[ "$MODE" != --build-only ]]; then
    TASK_APP_EXECUTABLE="$ROOT_DIR/dist/$TASK_APP_NAME.app/Contents/MacOS/$PRODUCT" python3 - <<'PYPROC'
import os,signal,subprocess,time
executable=os.environ['TASK_APP_EXECUTABLE']
for row in subprocess.check_output(['ps','-ax','-o','pid=,command='],text=True).splitlines():
    fields=row.strip().split(None,1)
    if len(fields)!=2: continue
    pid,command=fields
    if command!=executable and not command.startswith(executable+' '): continue
    os.kill(int(pid),signal.SIGTERM)
    for _ in range(30):
        try: os.kill(int(pid),0)
        except ProcessLookupError: break
        time.sleep(.1)
    else: raise SystemExit('Development app has not exited; build was not started')
PYPROC
fi
cargo build --manifest-path "$ROOT_DIR/Cargo.toml" --locked -p rmusic-mac-runtime
swift build --package-path "$ROOT_DIR" ${TASK_SWIFT_ARGS[@]+"${TASK_SWIFT_ARGS[@]}"} -j 2 --product "$PRODUCT"
SWIFT_BIN="$(swift build --package-path "$ROOT_DIR" ${TASK_SWIFT_ARGS[@]+"${TASK_SWIFT_ARGS[@]}"} --show-bin-path)"
BUNDLE="$TASK_DIST_DIR/$TASK_APP_NAME.app"
mkdir -p "$BUNDLE/Contents/"{MacOS,Helpers,Resources}
if [[ "$PRODUCT" == RMusicNative ]]; then
    cp -R "$SWIFT_BIN/RMusicMac_MineradioNative.bundle" "$BUNDLE/Contents/Resources/"
fi
cp "$SWIFT_BIN/$PRODUCT" "$BUNDLE/Contents/MacOS/$PRODUCT"
mkdir -p "$BUNDLE/Contents/Resources"
cp -R "$ROOT_DIR/VisualAssets/Sources/RMusicVisualAssets/Mineradio" "$BUNDLE/Contents/Resources/"
cp "$ROOT_DIR/AppIcon/RMusic.icns" "$BUNDLE/Contents/Resources/RMusic.icns"
TASK_CARGO_TARGET="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
cp "$TASK_CARGO_TARGET/debug/rmusic-mac-runtime" "$BUNDLE/Contents/Helpers/rmusic-mac-runtime"
cat > "$BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>$PRODUCT</string>
<key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
<key>CFBundleName</key><string>$TASK_APP_NAME</string>
<key>CFBundleIconFile</key><string>RMusic.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$TASK_VERSION</string>
<key>CFBundleVersion</key><string>${TASK_VERSION%%-*}</string>
<key>LSMinimumSystemVersion</key><string>27.0</string>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>LSUIElement</key><$MENU_ONLY/>
<key>NSBonjourServices</key><array><string>_rmusic-core._tcp</string><string>_rmusic-bridge._tcp</string></array>
<key>NSLocalNetworkUsageDescription</key><string>连接你指定的 R-Music Core 并接收本机播放指令。</string>
</dict></plist>
PLIST
# Development builds depend on the installed FFmpeg distribution. Finder needs explicit paths.
if [[ "${RMUSIC_MAC_PUBLIC:-0}" == 1 ]]; then
    strip -S "$BUNDLE/Contents/MacOS/$PRODUCT" "$BUNDLE/Contents/Helpers/rmusic-mac-runtime"
else
TASK_FFMPEG="$(command -v ffmpeg)" TASK_FFPROBE="$(command -v ffprobe)" TASK_TOOLS_FILE="$BUNDLE/Contents/Resources/development-tools.json" python3 - <<'PY'
import json,os,pathlib
pathlib.Path(os.environ['TASK_TOOLS_FILE']).write_text(json.dumps({'RMUSIC_FFMPEG':os.environ['TASK_FFMPEG'],'RMUSIC_FFPROBE':os.environ['TASK_FFPROBE']}))
PY
fi
TASK_SIGN_IDENTITY="${RMUSIC_MAC_SIGN_IDENTITY:--}"
codesign --force --sign "$TASK_SIGN_IDENTITY" --identifier "$BUNDLE_ID.runtime" "$BUNDLE/Contents/Helpers/rmusic-mac-runtime"
codesign --force --sign "$TASK_SIGN_IDENTITY" --identifier "$BUNDLE_ID" "$BUNDLE"
TASK_LAUNCH_ARGS=()
if [[ -n "${RMUSIC_MAC_QA_STDERR:-}" ]]; then
    if [[ "${RMUSIC_MAC_QA:-0}" != 1 ]]; then echo 'Diagnostic stderr is restricted to QA bundles' >&2; exit 2; fi
    TASK_LOG_ROOT="$ROOT_DIR/artifacts" python3 - <<'PYLOG'
import os,pathlib
p=pathlib.Path(os.environ['RMUSIC_MAC_QA_STDERR'])
root=pathlib.Path(os.environ['TASK_LOG_ROOT']).resolve()
if not p.is_absolute() or not p.resolve().is_relative_to(root): raise SystemExit('QA stderr must be inside macos/artifacts')
p.parent.mkdir(parents=True,exist_ok=True)
fd=os.open(p,os.O_WRONLY|os.O_CREAT|os.O_APPEND,0o600);os.close(fd);p.chmod(0o600)
PYLOG
    TASK_LAUNCH_ARGS=(--stderr "$RMUSIC_MAC_QA_STDERR" --env RMUSIC_AUDIO_DIAGNOSTICS=1)
fi
case "$MODE" in
    --build-only) echo "$BUNDLE";;
    --debug) lldb -- "$BUNDLE/Contents/MacOS/$PRODUCT";;
    --logs|--telemetry) open -n ${TASK_LAUNCH_ARGS[@]+"${TASK_LAUNCH_ARGS[@]}"} "$BUNDLE"; /usr/bin/log stream --info --style compact --predicate "process == \"$PRODUCT\"";;
    --verify) open -n ${TASK_LAUNCH_ARGS[@]+"${TASK_LAUNCH_ARGS[@]}"} "$BUNDLE"; sleep 1; pgrep -x "$PRODUCT";;
    run) open -n ${TASK_LAUNCH_ARGS[@]+"${TASK_LAUNCH_ARGS[@]}"} "$BUNDLE";;
esac
