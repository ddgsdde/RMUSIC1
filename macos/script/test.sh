#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TASK_SWIFT="$(xcrun --find swift)"
TASK_TEST_MACRO="$(dirname "$TASK_SWIFT")/../lib/swift/host/plugins/testing/libTestingMacros.dylib"
# macOS 27 CLT's SwiftPM does not always discover the Testing macro plugin itself.
TASK_ARGS=()
if [[ -f "$TASK_TEST_MACRO" ]]; then TASK_ARGS=(-Xswiftc -load-plugin-library -Xswiftc "$TASK_TEST_MACRO"); fi
swift test --package-path "$ROOT_DIR" -j 2 "${TASK_ARGS[@]}" "$@"
