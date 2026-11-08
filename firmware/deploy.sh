#!/usr/bin/env bash
# Usage: ./deploy.sh <bin> [--run] [-- args...]
#   HOST defaults to dreamteam@192.168.5.41 (radon board).
set -euo pipefail
cd "$(dirname "$0")"

bin="${1:?usage: ./deploy.sh <radon|servo> [--run] [-- args...]}"; shift
run=0
if [[ "${1:-}" == "--run" ]]; then run=1; shift; fi
[[ "${1:-}" == "--" ]] && shift

host="${HOST:-dreamteam@192.168.5.41}"
target=arm-unknown-linux-musleabihf

cargo build --release --bin "$bin"
scp -q "target/$target/release/$bin" "$host:~/$bin.new"
ssh "$host" "mv ~/$bin.new ~/$bin"
echo "deployed $bin -> $host:~/$bin"

if (( run )); then
    ssh -t "$host" "~/$bin $*"
fi
