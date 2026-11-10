SUMMARY = "CSE ${CSE_BOARD} board logger and A/B app updater (prebuilt static ARMv6 binaries from the signed GitHub release)"
LICENSE = "CLOSED"

CSE_REPO = "https://github.com/CSID-DGU/2026-1-CECD1-3-TeamDream-02"
CSE_TAG = "fw-v0.1.2"
# GitHub Actions run number of CSE_TAG (manifest.json "build"); the seed slot reports it so OTA only installs newer builds.
CSE_BUILD = "3"
# Boards poll this for newer signed releases (cse-ota.timer, hourly).
CSE_OTA_SOURCE = "${CSE_REPO}/releases/latest/download"

# The binaries are built and signed by .github/workflows/firmware-release.yml; Yocto only installs them,
# pinned by sha256 so the image content is identical on every build.
SRC_URI = " \
    ${CSE_REPO}/releases/download/${CSE_TAG}/${CSE_BOARD};name=bin;downloadfilename=${CSE_BOARD}-${CSE_TAG} \
    ${CSE_REPO}/releases/download/${CSE_TAG}/ota;name=ota;downloadfilename=ota-${CSE_TAG} \
    file://${CSE_BOARD}-logger.service \
    file://cse-ota.service \
    file://cse-ota.timer \
    file://cse-ota-seed.sh \
    file://cse-ota-seed.service \
"
SRC_URI[ota.sha256sum] = "eeab9d9dcc7a002aad580f58cecfacb03538c21e3b1a53585182d76ae1e8a538"

S = "${UNPACKDIR}"

# group i2c is created by udev-rules-rpi (meta-raspberrypi); it must exist in this recipe's sysroot for useradd
DEPENDS += "udev-rules-rpi"

inherit systemd useradd

SYSTEMD_SERVICE:${PN} = "${CSE_BOARD}-logger.service cse-ota-seed.service cse-ota.timer"
SYSTEMD_AUTO_ENABLE = "enable"

USERADD_PACKAGES = "${PN}"
USERADD_PARAM:${PN} = "--system --no-create-home --shell /sbin/nologin --groups i2c,dialout cse"

# ota fetches with curl over HTTPS
RDEPENDS:${PN} = "curl ca-certificates"

# static musl binaries: nothing to strip/split/link-check
INHIBIT_PACKAGE_STRIP = "1"
INHIBIT_PACKAGE_DEBUG_SPLIT = "1"
INSANE_SKIP:${PN} += "already-stripped ldflags arch file-rdeps"

do_install() {
    # seed for slot a; the logger runs /var/lib/cse-ota/current/<board>, never this copy
    install -d ${D}${libdir}/cse ${D}${bindir} ${D}${systemd_system_unitdir} ${D}${sysconfdir}/cse
    install -m 0755 ${UNPACKDIR}/${CSE_BOARD}-${CSE_TAG} ${D}${libdir}/cse/${CSE_BOARD}
    install -m 0755 ${UNPACKDIR}/ota-${CSE_TAG} ${D}${bindir}/ota

    install -m 0644 ${UNPACKDIR}/${CSE_BOARD}-logger.service ${UNPACKDIR}/cse-ota.service ${UNPACKDIR}/cse-ota.timer \
        ${D}${systemd_system_unitdir}/

    install -d ${D}${libexecdir}
    for f in cse-ota-seed.sh:${D}${libexecdir}/cse-ota-seed cse-ota-seed.service:${D}${systemd_system_unitdir}/cse-ota-seed.service; do
        sed -e 's|@BOARD@|${CSE_BOARD}|g' \
            -e 's|@SEED@|{"version":"${CSE_TAG}","build":${CSE_BUILD}}|g' \
            ${UNPACKDIR}/${f%%:*} > ${f#*:}
    done
    chmod 0755 ${D}${libexecdir}/cse-ota-seed

    cat > ${D}${sysconfdir}/cse/ota.conf <<EOF
board=${CSE_BOARD}
service=${CSE_BOARD}-logger.service
state_dir=/var/lib/cse-${CSE_BOARD}
source=${CSE_OTA_SOURCE}
EOF
}

FILES:${PN} += " \
    ${libdir}/cse \
    ${systemd_system_unitdir} \
    ${libexecdir}/cse-ota-seed \
    ${sysconfdir}/cse \
"
COMPATIBLE_MACHINE = "raspberrypi0-wifi"
