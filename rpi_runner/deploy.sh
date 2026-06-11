#!/usr/bin/env bash
# Cross-compile radon_sensor locally and deploy the binary to a Raspberry Pi.
# Building never happens on the device.
#
# Usage:
#   ./deploy.sh [user@]host          e.g. ./deploy.sh pi@192.168.1.42
#   RPI_HOST=pi@radon.local ./deploy.sh
#
# Required cross-toolchains (install once on your dev machine):
#   sudo apt install g++-arm-linux-gnueabihf       # Zero (1), Zero 2/2W 32-bit OS
#   sudo apt install g++-aarch64-linux-gnu         # Zero 2/2W 64-bit OS

set -euo pipefail

# ── Config ────────────────────────────────────────────────────────────────────
RPI_HOST="${1:-${RPI_HOST:-}}"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
SERVICE_DIR="/etc/systemd/system"
SSH_OPTS="-o StrictHostKeyChecking=accept-new -o ConnectTimeout=10"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── Preflight ─────────────────────────────────────────────────────────────────
if [[ -z "$RPI_HOST" ]]; then
    echo "Usage: $0 [user@]host  OR  export RPI_HOST=... and run $0" >&2
    exit 1
fi

# ── Detect Pi architecture ────────────────────────────────────────────────────
echo "==> Detecting target architecture on ${RPI_HOST}"
RPI_ARCH=$(ssh $SSH_OPTS "$RPI_HOST" "uname -m")
echo "    uname -m → ${RPI_ARCH}"

case "$RPI_ARCH" in
    armv6*)
        CROSS_COMPILE="arm-linux-gnueabihf-"
        ARCH_FLAGS="-march=armv6 -mfpu=vfp -mfloat-abi=hard"
        ;;
    armv7*)
        CROSS_COMPILE="arm-linux-gnueabihf-"
        ARCH_FLAGS="-march=armv8-a -mfpu=neon-fp-armv8 -mfloat-abi=hard"
        ;;
    aarch64)
        CROSS_COMPILE="aarch64-linux-gnu-"
        ARCH_FLAGS=""
        ;;
    *)
        echo "ERROR: Unrecognised architecture '${RPI_ARCH}'" >&2
        exit 1
        ;;
esac

echo "    Using CROSS_COMPILE=${CROSS_COMPILE} ARCH_FLAGS='${ARCH_FLAGS}'"

# Verify the toolchain is installed
if ! command -v "${CROSS_COMPILE}g++" &>/dev/null; then
    echo ""
    echo "ERROR: Cross-compiler '${CROSS_COMPILE}g++' not found." >&2
    echo "Install it with:" >&2
    if [[ "$CROSS_COMPILE" == aarch64* ]]; then
        echo "  sudo apt install g++-aarch64-linux-gnu" >&2
    else
        echo "  sudo apt install g++-arm-linux-gnueabihf" >&2
    fi
    exit 1
fi

# ── Build locally ─────────────────────────────────────────────────────────────
echo ""
echo "==> Building for ${RPI_ARCH}"
make -C "$SCRIPT_DIR" clean
make -C "$SCRIPT_DIR" -j"$(nproc)" \
    CROSS_COMPILE="$CROSS_COMPILE" \
    ARCH_FLAGS="$ARCH_FLAGS"

# ── Deploy ────────────────────────────────────────────────────────────────────
echo ""
echo "==> Copying binary and service file to ${RPI_HOST}"
scp $SSH_OPTS \
    "$SCRIPT_DIR/radon_sensor" \
    "$SCRIPT_DIR/radon-sensor.service" \
    "${RPI_HOST}:/tmp/"

echo "==> Installing"
ssh $SSH_OPTS "$RPI_HOST" "
    set -e
    sudo install -m 755 /tmp/radon_sensor '${INSTALL_DIR}/radon_sensor'
    id -u radon &>/dev/null || sudo useradd -r -s /sbin/nologin radon
    sudo usermod -aG dialout radon
    sudo install -m 644 /tmp/radon-sensor.service '${SERVICE_DIR}/radon-sensor.service'
    sudo systemctl daemon-reload
    sudo systemctl enable --now radon-sensor
    rm /tmp/radon_sensor /tmp/radon-sensor.service
"

echo ""
echo "==> Status"
ssh $SSH_OPTS "$RPI_HOST" \
    "systemctl is-active radon-sensor && journalctl -u radon-sensor -n 20 --no-pager || true"

echo ""
echo "Done. To watch live logs:"
echo "  ssh ${RPI_HOST} 'journalctl -fu radon-sensor'"
