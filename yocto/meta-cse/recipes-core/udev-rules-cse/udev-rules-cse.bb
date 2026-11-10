SUMMARY = "udev rules: /dev/serial0 for the ZD100 and i2c device permissions"
LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COREBASE}/meta/COPYING.MIT;md5=3da9cfbcb788c80a0384361b4de20420"

SRC_URI = "file://99-cse.rules"
S = "${UNPACKDIR}"

do_install() {
    install -d ${D}${sysconfdir}/udev/rules.d
    install -m 0644 ${UNPACKDIR}/99-cse.rules ${D}${sysconfdir}/udev/rules.d/
}
FILES:${PN} = "${sysconfdir}/udev/rules.d/99-cse.rules"
