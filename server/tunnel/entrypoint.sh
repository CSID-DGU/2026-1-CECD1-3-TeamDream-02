#!/bin/sh
# Host key lives in the volume: the boards pin it, so it must survive a new container.
set -eu
[ -f /data/ssh_host_ed25519_key ] || ssh-keygen -q -t ed25519 -N '' -f /data/ssh_host_ed25519_key
# sshd insists on root-owned key files; the mounted one belongs to whoever edits it on the host
install -m 0644 -o root -g root /run/cse/authorized_keys /etc/ssh/authorized_keys/tunnel
echo "host key: $(ssh-keygen -lf /data/ssh_host_ed25519_key.pub)"
echo "board keys: $(grep -c '^[^#]' /etc/ssh/authorized_keys/tunnel || true)"
exec /usr/sbin/sshd -D -e
