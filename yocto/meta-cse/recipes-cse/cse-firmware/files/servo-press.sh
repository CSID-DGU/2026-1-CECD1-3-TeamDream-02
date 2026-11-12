#!/bin/sh
# servo-press N [US]: press with servo N (1 or 2), optionally to US instead of the position in /etc/cse/servo.conf.
# Asks the running servo-logger, which does the motion and writes a "servo" record. Needs root or user cse.
exec /var/lib/cse-ota/current/servo press "$@"
