#!/bin/sh
# cse-servo-pwm setup|off, as root. Both servo signals off; "setup" also exports the channels and hands their
# controls to user cse, so the logger can press a button without being root.
P=/sys/class/pwm/pwmchip0
if [ "$1" = setup ]; then
    for c in 0 1; do
        [ -e $P/pwm$c ] || echo $c > $P/export
    done
    # udev needs a moment to create the attribute files
    n=0
    while [ ! -e $P/pwm1/enable ] && [ $n -lt 20 ]; do sleep 0.1; n=$((n + 1)); done
fi
for c in 0 1; do
    [ -e $P/pwm$c/enable ] || continue
    echo 0 > $P/pwm$c/enable
    [ "$1" = setup ] && chown cse $P/pwm$c/period $P/pwm$c/duty_cycle $P/pwm$c/enable
done
exit 0
