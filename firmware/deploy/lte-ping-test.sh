#!/bin/sh
# Switches wlan0 to another network known to wpa_supplicant (default: the LTE modem's hotspot), pings through it for
# a while and always switches back. Nothing is saved: a reboot also restores the normal network choice.
#
# Run on the board as root and detached, because the SSH session dies with the switch:
#   sudo setsid sh lte-ping-test.sh >/dev/null 2>&1 &
#   sudo setsid sh lte-ping-test.sh <test ssid> <home ssid> <seconds> >/dev/null 2>&1 &
# Result: /var/log/lte-ping-test.log (kept across reboots)
SSID=${1:-LDW931-K-312418}
HOME_SSID=${2:-YusiRouter}
DURATION=${3:-120}
IF=wlan0
LOG=/var/log/lte-ping-test.log
PING_TARGET=1.1.1.1
HTTPS_URL=https://radon.kafu.moe/healthz

log() { echo "$(date -u +%FT%TZ) $*" >> "$LOG"; }
cli() { wpa_cli -i "$IF" "$@"; }
id_of() { cli list_networks | awk -F'\t' -v s="$1" '$2 == s { print $1 }'; }
field() { cli status | sed -n "s/^$1=//p"; }

# connected to SSID $1 with an address and a default route, within $2 seconds
wait_for() {
    n=0
    while [ "$n" -lt "$2" ]; do
        if [ "$(field wpa_state)" = COMPLETED ] && [ "$(field ssid)" = "$1" ] && [ -n "$(field ip_address)" ] \
            && ip route | grep -q "^default.*$IF"; then
            return 0
        fi
        sleep 1
        n=$((n + 1))
    done
    return 1
}

visible() {
    for _ in 1 2 3; do
        cli scan > /dev/null
        sleep 5
        cli scan_results | awk -F'\t' -v s="$1" '$5 == s { found = 1 } END { exit !found }' && return 0
    done
    return 1
}

restore() {
    trap - EXIT INT TERM
    [ -n "$WATCHDOG" ] && kill "$WATCHDOG" 2> /dev/null
    log "switching back to $HOME_SSID"
    cli select_network "$HOME_ID" > /dev/null
    if wait_for "$HOME_SSID" 60; then
        log "back on $HOME_SSID, ip $(field ip_address)"
    else
        log "NOT back on $HOME_SSID after 60 s, restarting wpa_supplicant"
        systemctl restart "wpa_supplicant@$IF"
    fi
    # select_network disabled every other network: enable them again so priorities apply as before
    cli enable_network all > /dev/null
}

HOME_ID=$(id_of "$HOME_SSID")
TEST_ID=$(id_of "$SSID")
log "==== test of '$SSID' for ${DURATION}s, home '$HOME_SSID' (network ids $TEST_ID / $HOME_ID)"
if [ -z "$HOME_ID" ] || [ -z "$TEST_ID" ]; then
    log "abort: both networks must be in wpa_supplicant's list: $(cli list_networks | awk -F'\t' 'NR > 1 { print $2 }' | tr '\n' ' ')"
    exit 1
fi
if ! visible "$SSID"; then
    log "abort, not switching: '$SSID' is not in the scan results: $(cli scan_results | awk -F'\t' 'NR > 1 { print $5 "(" $3 ")" }' | tr '\n' ' ')"
    log "modem on USB: $(lsusb | grep -c 19d2:) device, $(dmesg | grep -c 'USB disconnect') USB disconnects since boot"
    exit 2
fi

# if this script is killed hard, still go back
(
    sleep $((DURATION + 150))
    cli select_network "$HOME_ID" > /dev/null
    sleep 30
    cli enable_network all > /dev/null
) &
WATCHDOG=$!
trap restore EXIT
trap 'exit 130' INT TERM

log "switching to $SSID"
cli select_network "$TEST_ID" > /dev/null
if ! wait_for "$SSID" 60; then
    log "could not join $SSID within 60 s: state $(field wpa_state), ssid '$(field ssid)', ip '$(field ip_address)'"
    exit 3
fi
log "joined $SSID: ip $(field ip_address), gateway $(ip route | awk '/^default/ { print $3; exit }'), $(iw dev "$IF" link | awk '/signal/ { print $2, $3 }')"

ok=0
fail=0
end=$(($(date +%s) + DURATION))
i=0
while [ "$(date +%s)" -lt "$end" ]; do
    if out=$(ping -c 1 -W 3 "$PING_TARGET" 2>&1); then
        ok=$((ok + 1))
        log "ping $PING_TARGET ok $(echo "$out" | sed -n 's/.*time=\([0-9.]* ms\).*/\1/p')"
    else
        fail=$((fail + 1))
        log "ping $PING_TARGET FAILED (ssid '$(field ssid)', state $(field wpa_state))"
    fi
    if [ $((i % 6)) -eq 0 ]; then
        log "https $HTTPS_URL: $(curl -sS -o /dev/null -m 15 -w '%{http_code} in %{time_total}s' "$HTTPS_URL" 2>&1 | tr '\n' ' ')"
    fi
    i=$((i + 1))
    sleep 5
done
log "result: $ok pings ok, $fail failed, $(dmesg | grep -c 'USB disconnect') USB disconnects since boot"
