SUMMARY = "CSE ${CSE_BOARD} board logger and A/B app updater (prebuilt static ARMv6 binaries from the signed GitHub release)"
LICENSE = "CLOSED"

CSE_REPO = "https://github.com/CSID-DGU/2026-1-CECD1-3-TeamDream-02"
CSE_TAG = "fw-v0.1.4"
# GitHub Actions run number of CSE_TAG (manifest.json "build"); the seed slot reports it so OTA only installs newer builds.
CSE_BUILD = "5"
# Boards poll this for newer signed releases (cse-ota.timer, hourly).
CSE_OTA_SOURCE = "${CSE_REPO}/releases/latest/download"
# Ingest server (server/ in this repo) the boards upload their records to.
CSE_UPLOAD_URL = "https://radon.kafu.moe/v1/records"

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
    file://cse-upload.service \
    file://cse-ota-run.sh \
    file://cse-request.sh \
    file://cse-request.path \
    file://cse-request.service \
    file://cse-tunnel.service \
    file://upload-token-${CSE_BOARD} \
"
# The board's upload token is a secret and lives outside git like the Wi-Fi password: CSE_PROVISION_DIR
# (default yocto/provision). "Unable to find file://upload-token-..." means it is missing there, see README.
FILESEXTRAPATHS:prepend := "${CSE_PROVISION_DIR}:"
SRC_URI[ota.sha256sum] = "e04b6bcacb18e96eefe71f928af4341806233e91a2c4c024f5c2a1b0c1ead124"

S = "${UNPACKDIR}"

# group i2c is created by udev-rules-rpi (meta-raspberrypi); it must exist in this recipe's sysroot for useradd
DEPENDS += "udev-rules-rpi"

inherit systemd useradd

SYSTEMD_SERVICE:${PN} = "${CSE_BOARD}-logger.service ${CSE_BOARD}-upload.service cse-ota-seed.service cse-ota.timer \
                         cse-request.path cse-tunnel.service"
SYSTEMD_AUTO_ENABLE = "enable"

USERADD_PACKAGES = "${PN}"
# gpio: the PWM controls in sysfs belong to that group (udev 99-com.rules), needed to press a button
USERADD_PARAM:${PN} = "--system --no-create-home --shell /sbin/nologin --groups i2c,dialout,gpio cse"

# ota and the uploader use curl over HTTPS
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
    for f in cse-ota-seed.sh:${D}${libexecdir}/cse-ota-seed cse-ota-seed.service:${D}${systemd_system_unitdir}/cse-ota-seed.service \
             cse-upload.service:${D}${systemd_system_unitdir}/${CSE_BOARD}-upload.service \
             cse-ota-run.sh:${D}${libexecdir}/cse-ota-run \
             cse-request.path:${D}${systemd_system_unitdir}/cse-request.path \
             cse-request.service:${D}${systemd_system_unitdir}/cse-request.service; do
        sed -e 's|@BOARD@|${CSE_BOARD}|g' \
            -e 's|@SEED@|{"version":"${CSE_TAG}","build":${CSE_BUILD}}|g' \
            ${UNPACKDIR}/${f%%:*} > ${f#*:}
    done
    chmod 0755 ${D}${libexecdir}/cse-ota-seed ${D}${libexecdir}/cse-ota-run
    install -m 0755 ${UNPACKDIR}/cse-request.sh ${D}${libexecdir}/cse-request
    # stays off until /etc/cse/tunnel.conf, tunnel_key and tunnel_known_hosts are put on the board (README)
    install -m 0644 ${UNPACKDIR}/cse-tunnel.service ${D}${systemd_system_unitdir}/

    cat > ${D}${sysconfdir}/cse/ota.conf <<EOF
board=${CSE_BOARD}
service=${CSE_BOARD}-logger.service
state_dir=/var/lib/cse-${CSE_BOARD}
source=${CSE_OTA_SOURCE}
EOF

    # readable by the uploader (user cse) only; the token never appears on a command line
    install -m 0640 -g cse ${UNPACKDIR}/upload-token-${CSE_BOARD} ${D}${sysconfdir}/cse/upload.token
    cat > ${D}${sysconfdir}/cse/upload.conf <<EOF
url=${CSE_UPLOAD_URL}
token_file=${sysconfdir}/cse/upload.token
EOF
}

FILES:${PN} += " \
    ${libdir}/cse \
    ${systemd_system_unitdir} \
    ${libexecdir}/cse-ota-seed \
    ${libexecdir}/cse-ota-run \
    ${libexecdir}/cse-request \
    ${sysconfdir}/cse \
"
COMPATIBLE_MACHINE = "raspberrypi0-wifi"
