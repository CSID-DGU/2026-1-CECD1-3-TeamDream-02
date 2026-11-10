FILESEXTRAPATHS:prepend := "${THISDIR}/files:"
SRC_URI += "file://50-cse-journal.conf file://50-cse-timesync.conf"

do_install:append() {
    install -d ${D}${sysconfdir}/systemd/journald.conf.d ${D}${sysconfdir}/systemd/timesyncd.conf.d
    install -m 0644 ${UNPACKDIR}/50-cse-journal.conf ${D}${sysconfdir}/systemd/journald.conf.d/
    install -m 0644 ${UNPACKDIR}/50-cse-timesync.conf ${D}${sysconfdir}/systemd/timesyncd.conf.d/
}

FILES:${PN} += "${sysconfdir}/systemd/journald.conf.d ${sysconfdir}/systemd/timesyncd.conf.d"
