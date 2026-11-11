#!/usr/bin/env bash
# Brings a running (already flashed) board up to what the current image recipes install, without reflashing:
# servo press permission and unit, hardware watchdog, read-only /boot, the request helper, the updater wrapper and
# the maintenance tunnel. Safe to run again. Reboot the board afterwards.
#   ./apply-to-board.sh dreamteam@192.168.5.41 [ssh options...]
# The tunnel stays off until it is told where to go (see server/tunnel/README.md):
#   TUNNEL_HOST=radon.kafu.moe TUNNEL_PORT=22022 TUNNEL_KNOWN_HOSTS=known_hosts ./apply-to-board.sh ...
# TUNNEL_KNOWN_HOSTS is a file with the tunnel server's host key line ("[host]:port ssh-ed25519 ...").
set -euo pipefail
host=${1:?usage: ./apply-to-board.sh user@host [ssh options...]}; shift
kh=""
if [ -n "${TUNNEL_HOST:-}" ]; then
    kh=$(realpath "${TUNNEL_KNOWN_HOSTS:?TUNNEL_KNOWN_HOSTS: file with the tunnel server host key line}")
fi
prov=$(cd "$(dirname "$0")" && pwd)/provision
cd "$(dirname "$0")/meta-cse"
F=recipes-cse/cse-firmware/files

tar -C "$F" -cf - cse-ota.service cse-ota-run.sh cse-request.sh cse-request.path cse-request.service \
    cse-tunnel.service servo-logger.service cse-servo-pwm.sh servo-press.sh servo.conf \
    -C "$PWD/recipes-core/systemd/files" 50-cse-watchdog.conf |
    ssh "$@" "$host" 'rm -rf /tmp/cse-apply && mkdir /tmp/cse-apply && tar -C /tmp/cse-apply -xf -'
if [ -n "$kh" ]; then
    grep -q "^\[$TUNNEL_HOST\]:${TUNNEL_PORT:?TUNNEL_PORT} ssh-" "$kh" || { echo "$kh has no line for [$TUNNEL_HOST]:$TUNNEL_PORT"; exit 1; }
    { echo "TUNNEL_HOST=$TUNNEL_HOST"; echo "TUNNEL_PORT=$TUNNEL_PORT"; echo "TUNNEL_USER=${TUNNEL_USER:-tunnel}"; } |
        ssh "$@" "$host" 'cat > /tmp/cse-apply/tunnel.conf'
    ssh "$@" "$host" 'cat > /tmp/cse-apply/tunnel_known_hosts' < "$kh"
fi
# the board's tunnel key from provision/ (the one the image would carry), if there is one for this board
b=$(ssh "$@" "$host" hostname)
if [ -f "$prov/tunnel-key-$b" ]; then
    ssh "$@" "$host" 'umask 077; cat > /tmp/cse-apply/tunnel_key' < "$prov/tunnel-key-$b"
    ssh "$@" "$host" 'cat > /tmp/cse-apply/tunnel_key.pub' < "$prov/tunnel-key-$b.pub"
fi
ssh "$@" "$host" 'sudo sh -s' <<'REMOTE'
set -eu
cd /tmp/cse-apply
b=$(hostname)
U=/usr/lib/systemd/system
put() { cp "$1" "$2" && chown root:root "$2" && chmod "$3" "$2"; }           # file, target, mode
fill() { sed "s|@BOARD@|$b|g" "$1" > "$2" && chown root:root "$2" && chmod "$3" "$2"; }
join() {                                                                     # user, group
    grep -q "^$2:" /etc/group || { echo "group $2 is missing"; exit 1; }
    awk -F: -v u="$1" -v g="$2" 'BEGIN { OFS = ":" }
        $1 == g && ("," $4 ",") !~ ("," u ",") { $4 = ($4 == "" ? u : $4 "," u) } { print }' /etc/group > /etc/group.new
    cat /etc/group.new > /etc/group && rm /etc/group.new
}

# request helper and updater wrapper
put cse-request.sh /usr/libexec/cse-request 755
fill cse-request.path $U/cse-request.path 644
fill cse-request.service $U/cse-request.service 644
fill cse-ota-run.sh /usr/libexec/cse-ota-run 755
put cse-ota.service $U/cse-ota.service 644
put cse-tunnel.service $U/cse-tunnel.service 644

# hardware watchdog, read-only /boot
mkdir -p /etc/systemd/system.conf.d
put 50-cse-watchdog.conf /etc/systemd/system.conf.d/50-cse-watchdog.conf 644
sed -i 's|^\(/dev/mmcblk0p1[[:space:]]*/boot[[:space:]]*vfat[[:space:]]*\)defaults|\1ro|' /etc/fstab

join dreamteam systemd-journal
join cse gpio
if [ "$b" = servo ]; then
    put servo-logger.service $U/servo-logger.service 644
    put cse-servo-pwm.sh /usr/libexec/cse-servo-pwm 755
    put servo-press.sh /usr/bin/servo-press 755
    [ -e /etc/cse/servo.conf ] || put servo.conf /etc/cse/servo.conf 644
fi

# key for the maintenance tunnel: the provisioned one, else one made here
if [ -e tunnel_key ]; then
    cp tunnel_key /etc/cse/tunnel_key && cp tunnel_key.pub /etc/cse/tunnel_key.pub
    chown cse:cse /etc/cse/tunnel_key && chmod 400 /etc/cse/tunnel_key
elif [ ! -e /etc/cse/tunnel_key ]; then
    ssh-keygen -q -t ed25519 -N '' -C "cse-$b-tunnel" -f /etc/cse/tunnel_key
    chown cse:cse /etc/cse/tunnel_key && chmod 400 /etc/cse/tunnel_key
fi

# where the tunnel goes; each board gets its own port on the server (radon 22041, servo 22042)
if [ -e tunnel.conf ]; then
    case "$b" in radon) rport=22041 ;; servo) rport=22042 ;; *) echo "no tunnel port for board $b"; exit 1 ;; esac
    { cat tunnel.conf; echo "TUNNEL_RPORT=$rport"; } > /etc/cse/tunnel.conf
    put tunnel_known_hosts /etc/cse/tunnel_known_hosts 644
    chmod 644 /etc/cse/tunnel.conf
fi

systemctl daemon-reload
systemctl enable cse-request.path cse-tunnel.service > /dev/null 2>&1
sync
rm -rf /tmp/cse-apply
echo "applied on $b. boot line: $(grep /boot /etc/fstab)"
echo "groups: $(id cse | sed 's/.*groups=//') | $(id dreamteam | sed 's/.*groups=//')"
echo "tunnel: $([ -e /etc/cse/tunnel.conf ] && tr '\n' ' ' < /etc/cse/tunnel.conf || echo 'not configured (off)')"
echo "tunnel public key: $(cat /etc/cse/tunnel_key.pub)"
REMOTE
