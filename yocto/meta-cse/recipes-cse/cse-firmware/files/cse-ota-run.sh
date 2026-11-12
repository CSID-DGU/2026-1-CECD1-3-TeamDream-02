#!/bin/sh
# Runs the updater. /usr/bin/ota is part of the image and is never replaced by an update, so a board binary that
# brings its own updater ("<board> ota-version" succeeds) is preferred: that is how the updater itself can be
# updated. If it fails, the image's updater still runs.
b=/var/lib/cse-ota/current/@BOARD@
if [ -x "$b" ] && "$b" ota-version > /dev/null 2>&1; then
    "$b" ota run && exit 0
    echo "cse-ota-run: the board binary's updater failed, running the image's"
fi
exec /usr/bin/ota run
