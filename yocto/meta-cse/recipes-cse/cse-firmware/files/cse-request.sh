#!/bin/sh
# Root helper for the board programs. They run unprivileged and sandboxed and cannot use sudo, so they ask by
# writing ONE word into <state dir>/request (cse-request.path starts this). Only the actions below exist;
# anything else is dropped. A future firmware can use this, e.g. to recover from a dead uplink.
f=$1
if [ -L "$f" ] || [ ! -f "$f" ]; then
    rm -f "$f"
    exit 0
fi
# first line only, at most 32 characters (busybox head has no -c)
IFS= read -r word < "$f" || true
word=$(printf '%s' "$word" | cut -c1-32 | tr -cd 'a-z-')
rm -f "$f"
case "$word" in
    restart-wifi)
        echo "cse-request: restarting wpa_supplicant on wlan0"
        systemctl restart wpa_supplicant@wlan0.service
        ;;
    restart-network)
        echo "cse-request: restarting systemd-networkd"
        systemctl restart systemd-networkd.service
        ;;
    reboot)
        # at most one requested reboot per 30 minutes of uptime, so a faulty program cannot keep the board down
        up=$(cut -d. -f1 /proc/uptime)
        if [ "$up" -lt 1800 ]; then
            echo "cse-request: reboot refused, up for only ${up}s (allowed 30 min after boot)"
        else
            echo "cse-request: rebooting"
            systemctl reboot
        fi
        ;;
    *)
        echo "cse-request: unknown request dropped"
        ;;
esac
exit 0
