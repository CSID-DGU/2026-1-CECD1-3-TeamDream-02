CSE_BOARD = "servo"
SRC_URI[bin.sha256sum] = "6c5aba67f0e36e75b0d184377f6fcf2011788ec3bfc4e5d488c464c21d91d1fc"
require cse-firmware.inc

SRC_URI += "file://cse-servo-pwm.sh file://servo.conf file://servo-press.sh"

do_install:append() {
    install -m 0755 ${UNPACKDIR}/cse-servo-pwm.sh ${D}${libexecdir}/cse-servo-pwm
    install -m 0755 ${UNPACKDIR}/servo-press.sh ${D}${bindir}/servo-press
    install -m 0644 ${UNPACKDIR}/servo.conf ${D}${sysconfdir}/cse/servo.conf
}

FILES:${PN} += "${libexecdir}/cse-servo-pwm"
CONFFILES:${PN} += "${sysconfdir}/cse/servo.conf"
