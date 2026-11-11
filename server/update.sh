#!/bin/sh
# Installs the newest published server image (GitHub release server-v*) and restarts the container if it is newer
# than the one compose.yaml names. Run it in the directory with compose.yaml, by hand or from cron, e.g. hourly:
#   17 * * * *  /home/me/cse/update.sh >> /home/me/cse/update.log 2>&1
# Needs curl, zstd, sha256sum and docker compose. If the new container does not become healthy within a minute,
# the previous image is started again.
set -eu
REPO=CSID-DGU/2026-1-CECD1-3-TeamDream-02
cd "$(dirname "$0")"
say() { echo "$(date -u +%FT%TZ) $*"; }

new=$(curl -fsSL "https://api.github.com/repos/$REPO/releases?per_page=50" \
    | grep -o '"tag_name": *"server-v[^"]*"' | head -n 1 | sed 's/.*"server-v\(.*\)"/\1/')
echo "$new" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || { say "no server-v* release found"; exit 1; }
cur=$(sed -n 's/^ *image: *cse-server:\(.*\)$/\1/p' compose.yaml | head -n 1)
[ "$new" = "$cur" ] && exit 0

say "updating cse-server $cur -> $new"
tmp=$(mktemp -d)
trap 'rm -r "$tmp"' EXIT
f="cse-server-$new.tar.zst"
curl -fsSL -o "$tmp/$f" "https://github.com/$REPO/releases/download/server-v$new/$f"
curl -fsSL -o "$tmp/$f.sha256" "https://github.com/$REPO/releases/download/server-v$new/$f.sha256"
(cd "$tmp" && sha256sum -c "$f.sha256" > /dev/null)
zstd -dc "$tmp/$f" | docker load > /dev/null

healthy() {
    n=0
    while [ "$n" -lt 30 ]; do
        [ "$(docker inspect -f '{{.State.Health.Status}}' "$(docker compose ps -q cse-server)" 2> /dev/null)" = healthy ] && return 0
        sleep 2
        n=$((n + 1))
    done
    return 1
}
use() { sed -i "s/^\( *image: *cse-server:\).*/\1$1/" compose.yaml && docker compose up -d > /dev/null 2>&1; }

if ! use "$new"; then
    say "failed to start cse-server $new, going back to $cur"
    use "$cur"
    exit 1
fi
if healthy; then
    say "cse-server $new is running"
else
    say "cse-server $new did not become healthy, going back to $cur"
    use "$cur"
    exit 1
fi
