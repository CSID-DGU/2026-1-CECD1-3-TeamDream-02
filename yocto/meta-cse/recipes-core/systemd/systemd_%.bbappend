FILESEXTRAPATHS:prepend := "${THISDIR}/files:"
SRC_URI += "file://50-cse-journal.conf"

do_install:append() {
    install -d ${D}${sysconfdir}/systemd/journald.conf.d
    install -m 0644 ${UNPACKDIR}/50-cse-journal.conf ${D}${sysconfdir}/systemd/journald.conf.d/
}

FILES:${PN} += "${sysconfdir}/systemd/journald.conf.d"
