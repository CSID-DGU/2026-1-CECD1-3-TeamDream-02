FILESEXTRAPATHS:prepend := "${THISDIR}/files:"
SRC_URI += "file://50-cse-journal.conf file://50-cse-timesync.conf file://50-cse-watchdog.conf"

do_install:append() {
    install -d ${D}${sysconfdir}/systemd/journald.conf.d ${D}${sysconfdir}/systemd/timesyncd.conf.d \
        ${D}${sysconfdir}/systemd/system.conf.d
    install -m 0644 ${UNPACKDIR}/50-cse-watchdog.conf ${D}${sysconfdir}/systemd/system.conf.d/
    install -m 0644 ${UNPACKDIR}/50-cse-journal.conf ${D}${sysconfdir}/systemd/journald.conf.d/
    install -m 0644 ${UNPACKDIR}/50-cse-timesync.conf ${D}${sysconfdir}/systemd/timesyncd.conf.d/
}

FILES:${PN} += "${sysconfdir}/systemd/journald.conf.d ${sysconfdir}/systemd/timesyncd.conf.d ${sysconfdir}/systemd/system.conf.d"
