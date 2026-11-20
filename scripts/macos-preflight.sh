#!/usr/bin/env bash
# Read-only toolchain inventory; does not install, build, launch, or open audio devices.
set -u
set -o pipefail
if [[ "$(uname -s)" != "Darwin" ]]; then
  printf '%s\n' '请在 M5 Mac 的项目根目录运行此检查；当前不是 macOS。' >&2
  exit 2
fi
missing=0
printf 'Architecture: %s\n' "$(uname -m)"
sw_vers
if ! xcode-select -p; then missing=1; fi
if ! xcrun --sdk macosx --show-sdk-path; then missing=1; fi
for tool in rustc cargo node npm ffmpeg ffprobe git; do
  if command -v "$tool" >/dev/null 2>&1; then
    printf '\n%s: ' "$tool"
    if [[ "$tool" == "ffmpeg" || "$tool" == "ffprobe" ]]; then version_flag=-version; else version_flag=--version; fi
    if ! "$tool" "$version_flag" 2>&1 | sed -n '1p'; then missing=1; fi
  else
    printf '\nMissing: %s\n' "$tool"
    missing=1
  fi
done
if command -v rustup >/dev/null 2>&1; then
  printf '\nInstalled Rust targets:\n'
  rustup target list --installed
fi
printf '\nGit workspace:\n'
git rev-parse --show-toplevel 2>/dev/null || true
exit "$missing"
