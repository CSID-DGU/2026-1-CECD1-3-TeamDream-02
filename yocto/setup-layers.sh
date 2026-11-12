#!/usr/bin/env bash
# Clone (or reset) every layer to the revision pinned in layers.lock. meta-cse lives here and is not cloned.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p layers
while read -r dir url branch rev; do
  [ -d "layers/$dir/.git" ] || git clone -q --single-branch -b "$branch" "$url" "layers/$dir"
  git -C "layers/$dir" fetch -q origin "$branch"
  git -C "layers/$dir" checkout -q "$rev"
  echo "$dir @ ${rev:0:10}"
done < layers.lock
