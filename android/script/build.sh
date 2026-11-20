#!/usr/bin/env bash
set -euo pipefail
TASK_ANDROID_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export JAVA_HOME="${JAVA_HOME:-/Applications/Android Studio.app/Contents/jbr/Contents/Home}"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
cd "$TASK_ANDROID_ROOT"
exec ./gradlew --no-daemon --max-workers=2 "${@:-:app:assembleDebug}"
