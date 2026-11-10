#!/usr/bin/env bash
# Install the pinned Yocto buildtools (host gcc/python/git/... independent of the distro) into ./buildtools.
set -euo pipefail
cd "$(dirname "$0")"

VER=6.0.4
FILE=x86_64-buildtools-extended-nativesdk-standalone-$VER.sh
URL=https://downloads.yoctoproject.org/releases/yocto/yocto-$VER/buildtools/$FILE
SHA256=93002cc067c15523c1fca4e2ee047a1dba0eed7ccfdb41652141741edb5469b2

[ -f buildtools/environment-setup-x86_64-pokysdk-linux ] && { echo "buildtools $VER already installed"; exit 0; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/$FILE" "$URL"
echo "$SHA256  $tmp/$FILE" | sha256sum -c -
bash "$tmp/$FILE" -y -d "$PWD/buildtools" >/dev/null
echo "buildtools $VER installed"
