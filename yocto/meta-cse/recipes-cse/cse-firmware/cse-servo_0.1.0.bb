CSE_BOARD = "servo"
SRC_URI[bin.sha256sum] = "11fafcba0841bfeb9aa8a3d5d11145361e1c41f15da2ebbb41ba0aee5026286f"
require cse-firmware.inc

SRC_URI += "file://cse-servo-pwm.sh file://servo.conf file://servo-press.sh"

do_install:append() {
    install -m 0755 ${UNPACKDIR}/cse-servo-pwm.sh ${D}${libexecdir}/cse-servo-pwm
    install -m 0755 ${UNPACKDIR}/servo-press.sh ${D}${bindir}/servo-press
    install -m 0644 ${UNPACKDIR}/servo.conf ${D}${sysconfdir}/cse/servo.conf
}

FILES:${PN} += "${libexecdir}/cse-servo-pwm"
CONFFILES:${PN} += "${sysconfdir}/cse/servo.conf"
