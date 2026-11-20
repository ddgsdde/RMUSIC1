#!/usr/bin/env bash
# Isolated local Core development. Never imports the remote server's data or provider login.
set -euo pipefail
TASK_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
[[ "$(uname -s)" == Darwin ]] || { echo '此入口用于 macOS 开发。' >&2; exit 2; }
if ! command -v cargo >/dev/null; then export PATH="$(dirname "$(rustup which rustc)"):$PATH"; fi
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:$PATH"
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2
TASK_DATA_ROOT="${RMUSIC_DEV_DATA_ROOT:-$TASK_ROOT/artifacts/development/core-macos}"
export RMUSIC_DATA_DIR="$TASK_DATA_ROOT/data"
export RMUSIC_CACHE_DIR="$TASK_DATA_ROOT/cache"
export RMUSIC_PLUGIN_DIR="$TASK_DATA_ROOT/plugins"
export RMUSIC_BUILTIN_PLUGIN_DIR="$TASK_DATA_ROOT/empty-plugins"
export RMUSIC_CACHE_ENABLED=0
export RMUSIC_BIND=127.0.0.1 RMUSIC_PORT="${RMUSIC_DEV_PORT:-18864}" RMUSIC_DISCOVERY=0
export RMUSIC_FFMPEG="$(command -v ffmpeg)" RMUSIC_FFPROBE="$(command -v ffprobe)"
mkdir -p "$RMUSIC_DATA_DIR" "$RMUSIC_CACHE_DIR" "$RMUSIC_PLUGIN_DIR" "$RMUSIC_BUILTIN_PLUGIN_DIR"
TASK_BUILD=1
TASK_WEB=0
for TASK_ARG in "$@"; do
    case "$TASK_ARG" in --with-web) TASK_WEB=1;; --no-build) TASK_BUILD=0;; *) echo "未知开发选项" >&2; exit 2;; esac
done
if [[ "$TASK_WEB" == 1 ]]; then
    (cd "$TASK_ROOT/apps/web" && npm run build)
    export RMUSIC_WEB_DIR="$TASK_ROOT/apps/web/dist"
else
    export RMUSIC_WEB_DIR="$RMUSIC_BUILTIN_PLUGIN_DIR"
fi
# A dedicated target directory keeps server development independent of client packaging.
export CARGO_TARGET_DIR="$TASK_ROOT/macos/target"
if [[ "$TASK_BUILD" == 1 ]]; then cargo build --manifest-path "$TASK_ROOT/Cargo.toml" --locked -p rmusic-core; fi
[[ -x "$CARGO_TARGET_DIR/debug/rmusic-core" ]] || { echo "缺少开发 Core 程序" >&2; exit 2; }
printf '开发 Core: http://127.0.0.1:%s (独立开发数据目录)\n' "$RMUSIC_PORT"
exec "$CARGO_TARGET_DIR/debug/rmusic-core"
