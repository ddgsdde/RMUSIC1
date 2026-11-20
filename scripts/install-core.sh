#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
usage() {
    printf '%s\n' 'Usage: scripts/install-core.sh [--nas] [--build | --image ARCHIVE]'
}

compose_file=compose.yaml
env_file=.env
archive=
build=false
while (($#)); do
    case "$1" in
        --nas) compose_file=compose.nas.yaml; env_file=.env.nas ;;
        --build) build=true ;;
        --image)
            if (($# < 2)); then usage >&2; exit 2; fi
            archive="$2"
            shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
    shift
done
if [[ "$build" == true && -n "$archive" ]]; then usage >&2; exit 2; fi

if [[ ! -f "$root/$env_file" ]]; then
    cp "$root/$env_file.example" "$root/$env_file"
    if [[ "$env_file" == .env.nas ]]; then
        printf '请在 %s 设置 NAS 的局域网地址和音乐目录，然后重新运行。\n' "$root/$env_file"
        exit 2
    fi
fi

if [[ -n "$archive" ]]; then docker load --input "$archive"; fi
mkdir -p "$root/music"
compose=(docker compose --project-directory "$root" --env-file "$root/$env_file" -f "$root/$compose_file")
"${compose[@]}" config --quiet
if [[ "$build" == true ]]; then
    "${compose[@]}" build core
fi
"${compose[@]}" up --detach --no-build --pull never --wait core
"${compose[@]}" ps core
