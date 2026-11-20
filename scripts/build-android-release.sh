#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
version="${1:-0.1.0-preview.1}"
version_code="${2:-10001}"
: "${ANDROID_HOME:?Set ANDROID_HOME to your Android SDK}"
: "${RMUSIC_ANDROID_KEYSTORE:?Set RMUSIC_ANDROID_KEYSTORE to your PKCS#12 signing key}"
: "${RMUSIC_ANDROID_KEY_PASSWORD:?Set RMUSIC_ANDROID_KEY_PASSWORD in your environment}"
out="${RMUSIC_ANDROID_DIST:-$root/releases}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
export JAVA_HOME NDK_HOME ANDROID_HOME
export RMUSIC_ANDROID_APPLICATION_ID=com.rmusic.android
export RMUSIC_ANDROID_APP_NAME=R-Music
export RMUSIC_ANDROID_VERSION_CODE="$version_code"
export RMUSIC_ANDROID_VERSION_NAME="$version"
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2
separator=$'\x1f'
export CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS:+$CARGO_ENCODED_RUSTFLAGS$separator}--remap-path-prefix=$HOME=/build-user$separator--remap-path-prefix=$root=/src"
sdk_tools="$ANDROID_HOME/build-tools/36.0.0"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
npm --prefix "$root/apps/web" ci --no-audit --no-fund
npm --prefix "$root/apps/web" run build
npm --prefix "$root/apps/desktop" ci --no-audit --no-fund
(
    cd "$root/apps/desktop"
    node node_modules/@tauri-apps/cli/tauri.js android build --apk --ci --target aarch64 --features tauri-app,adaptive-sync
)
apk="$(find "$root/apps/desktop/src-tauri/gen/android/app/build/outputs/apk" -path '*/release/*.apk' -type f -print -quit)"
test -n "$apk"
"$sdk_tools/zipalign" -f -P 16 4 "$apk" "$work/aligned.apk"
"$sdk_tools/apksigner" sign --ks "$RMUSIC_ANDROID_KEYSTORE" --ks-type PKCS12 \
    --ks-pass env:RMUSIC_ANDROID_KEY_PASSWORD --out "$out/R-Music-Android-arm64.apk" "$work/aligned.apk"
"$sdk_tools/apksigner" verify --verbose "$out/R-Music-Android-arm64.apk"
"$sdk_tools/zipalign" -c -P 16 4 "$out/R-Music-Android-arm64.apk"
