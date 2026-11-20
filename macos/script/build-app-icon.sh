#!/usr/bin/env bash
set -euo pipefail
TASK_ICON_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../AppIcon" && pwd)"
TASK_ICON_WORK="$(mktemp -d "${TMPDIR:-/tmp}/rmusic-icon.XXXXXX")"
mkdir -p "$TASK_ICON_WORK/RMusic.iconset"
for size in 16 32 128 256 512; do
  rsvg-convert -w "$size" -h "$size" "$TASK_ICON_ROOT/RMusicIcon.svg" -o "$TASK_ICON_WORK/RMusic.iconset/icon_${size}x${size}.png"
  double=$((size * 2))
  rsvg-convert -w "$double" -h "$double" "$TASK_ICON_ROOT/RMusicIcon.svg" -o "$TASK_ICON_WORK/RMusic.iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$TASK_ICON_WORK/RMusic.iconset" -o "$TASK_ICON_ROOT/RMusic.icns"
rsvg-convert -w 1024 -h 1024 "$TASK_ICON_ROOT/RMusicIcon.svg" -o "$TASK_ICON_ROOT/RMusicIcon.png"
printf '%s\n' "$TASK_ICON_ROOT/RMusic.icns"
