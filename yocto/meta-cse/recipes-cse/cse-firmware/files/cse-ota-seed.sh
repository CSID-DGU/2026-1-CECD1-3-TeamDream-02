#!/bin/sh
# Seeds slot a of /var/lib/cse-ota from the binary shipped in the image: on first boot, and again whenever the
# active slot is empty (a power cut before the first write reached the SD card leaves 0-byte files behind).
# @BOARD@ and @SEED@ are filled in by the recipe.
set -e
R=/var/lib/cse-ota
B=@BOARD@
[ -s "$R/current/$B" ] && [ -s "$R/current/build.json" ] && exit 0

echo "seeding slot a with the image's $B:" '@SEED@'
mkdir -p "$R/slots/a" "$R/slots/b" "$R/ota"
cp "/usr/lib/cse/$B" "$R/slots/a/$B.new"
chmod 0755 "$R/slots/a/$B.new"
mv "$R/slots/a/$B.new" "$R/slots/a/$B"
printf '%s' '@SEED@' > "$R/slots/a/build.json.new"
mv "$R/slots/a/build.json.new" "$R/slots/a/build.json"
rm -f "$R/current" "$R/ota/pending.json"
ln -s slots/a "$R/current"
ln -sf /etc/cse/ota.conf "$R/ota.conf"
chown -R cse:cse "$R"
sync
