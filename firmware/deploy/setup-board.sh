#!/usr/bin/env bash
# One-time board setup (asks for sudo on the Pi once). After this, updates arrive by OTA.
# Usage: OTA_SOURCE=https://github.com/CSID-DGU/2026-1-CECD1-3-TeamDream-02/releases/latest/download \
#        HOST=dreamteam@<ip> ./deploy/setup-board.sh radon|servo
set -euo pipefail
cd "$(dirname "$0")/.."

board="${1:?usage: setup-board.sh radon|servo}"
case "$board" in radon|servo) ;; *) echo "board must be radon or servo" >&2; exit 2 ;; esac
host="${HOST:-dreamteam@192.168.5.41}"
source="${OTA_SOURCE:?set OTA_SOURCE to the release download URL}"
arm=target/arm-unknown-linux-musleabihf/release

cargo build --release --bins
scp -q "$arm/$board" "$arm/ota" "deploy/units/$board-logger.service" deploy/units/cse-ota.service deploy/units/cse-ota.timer "$host:/tmp/"
ssh -t "$host" "BOARD=$board SOURCE='$source' bash -s" <<'EOF'
set -euo pipefail
R=$HOME/cse
mkdir -p "$R/slots/a" "$R/slots/b" "$R/ota" "$R/bin"
install -m 755 /tmp/ota "$R/bin/ota"
if [ ! -L "$R/current" ]; then
  install -m 755 "/tmp/$BOARD" "$R/slots/a/$BOARD"
  echo '{"version":"initial","build":0}' > "$R/slots/a/build.json"
  ln -s slots/a "$R/current"
else
  echo "current slot exists ($(readlink "$R/current")): app left to OTA"
fi
cat > "$R/ota.conf" <<CONF
board=$BOARD
service=$BOARD-logger.service
state_dir=/var/lib/cse-$BOARD
source=$SOURCE
CONF
sudo install -m 644 "/tmp/$BOARD-logger.service" /tmp/cse-ota.service /tmp/cse-ota.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now "$BOARD-logger.service" cse-ota.timer
rm -f "/tmp/$BOARD" /tmp/ota /tmp/*-logger.service /tmp/cse-ota.service /tmp/cse-ota.timer
sleep 3
systemctl --no-pager --lines=3 status "$BOARD-logger.service" || true
CSE_ROOT=$R "$R/bin/ota" status
EOF
