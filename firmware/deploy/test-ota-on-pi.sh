#!/usr/bin/env bash
# OTA end-to-end test on a Pi without sudo: user-scope logger, file:// source, short health window.
# Scenarios: install build 5 (healthy), tampered manifest (rejected), broken build 6 (rolled back).
set -euo pipefail
cd "$(dirname "$0")/.."
host="${HOST:-dreamteam@192.168.5.41}"
key="${OTA_SIGNING_KEY_FILE:-$HOME/.config/cse-ota/ota_signing.key}"
arm=target/arm-unknown-linux-musleabihf/release
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cargo build -q --release --bins
cargo build -q --release --target x86_64-unknown-linux-gnu --bin otatool
tool=target/x86_64-unknown-linux-gnu/release/otatool
cp "$arm/radon" "$work/radon-build0"

FW_VERSION=test-5 FW_BUILD=5 cargo build -q --release --bin radon
OTA_SIGNING_KEY_FILE="$key" "$tool" release --version test-5 --build 5 --out "$work/rel5" radon="$arm/radon" >/dev/null

mkdir -p "$work/bad"; cp -r "$work/rel5" "$work/bad/rel"
sed -i 's/"build": 5/"build": 50/' "$work/bad/rel/manifest.json"

printf '#!/bin/sh\nexit 1\n' > "$work/broken"
OTA_SIGNING_KEY_FILE="$key" "$tool" release --version test-6-broken --build 6 --out "$work/rel6" radon="$work/broken" >/dev/null
cargo build -q --release --bin radon

scp -q -r "$work/radon-build0" "$work/rel5" "$work/rel6" "$work/bad/rel" "$arm/ota" "$host:/tmp/"
ssh "$host" bash -s <<'EOF'
set -u
R=$HOME/cse-test
systemctl --user stop cse-test-logger.service 2>/dev/null; systemctl --user reset-failed cse-test-logger.service 2>/dev/null
rm -rf "$R"; mkdir -p "$R/slots/a" "$R/slots/b" "$R/ota" "$R/state" "$R/bin"
mv /tmp/radon-build0 "$R/slots/a/radon"; chmod 755 "$R/slots/a/radon"
echo '{"version":"dev","build":0}' > "$R/slots/a/build.json"
ln -s slots/a "$R/current"
mv /tmp/ota "$R/bin/ota"; chmod 755 "$R/bin/ota"
cat > "$R/ota.conf" <<CONF
board=radon
service=cse-test-logger.service
state_dir=$R/state
source=file:///tmp/rel5
systemctl_scope=user
health_window_s=60
CONF
systemd-run --user --unit=cse-test-logger -p Restart=always -p RestartSec=2 "$R/current/radon" log "$R/state" 10 >/dev/null
sleep 3
ota() { CSE_ROOT=$R "$R/bin/ota" "$@"; }

echo "== 1. tampered manifest (expect rejection)"; ota run --source file:///tmp/rel; echo "exit=$?"
echo "== 2. install build 5 (expect healthy)";      ota run; echo "exit=$?"
echo "== 3. same build again (expect up to date)";  ota run; echo "exit=$?"
echo "== 4. broken build 6 (expect rollback)";      ota run --source file:///tmp/rel6; echo "exit=$?"
sleep 15
echo "== 5. broken build 6 again (expect skip)";    ota run --source file:///tmp/rel6; echo "exit=$?"
echo "== status"; ota status
echo "== running binary: $("$R/current/radon" version)"
echo "== last record"; tail -n1 "$(ls "$R/state"/data-*.ndjson | tail -1)"

systemctl --user stop cse-test-logger.service; systemctl --user reset-failed cse-test-logger.service 2>/dev/null
rm -rf /tmp/rel /tmp/rel5 /tmp/rel6 "$R"
EOF
