SUMMARY = "Per-site access: dreamteam user with SSH key login, Wi-Fi (wpa_supplicant + networkd)"
LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COREBASE}/meta/COPYING.MIT;md5=3da9cfbcb788c80a0384361b4de20420"

# authorized_keys and wpa_supplicant-wlan0.conf are secrets-adjacent and live outside git:
# CSE_PROVISION_DIR (default yocto/provision), template in provision.example/.
# "Unable to find file://authorized_keys" means yocto/provision/ is missing: copy provision.example/ first.
FILESEXTRAPATHS:prepend := "${CSE_PROVISION_DIR}:"
SRC_URI = " \
    file://authorized_keys \
    file://wpa_supplicant-wlan0.conf \
    file://20-wlan0.network \
    file://90-cse-sudo \
"
S = "${UNPACKDIR}"

# group i2c is created by udev-rules-rpi (meta-raspberrypi)
DEPENDS += "udev-rules-rpi"

inherit useradd

USERADD_PACKAGES = "${PN}"
# password '*': no password can match, but the account is not locked. useradd's default '!' marks it locked,
# and sshd without PAM then refuses every login, SSH keys included.
USERADD_PARAM:${PN} = "--create-home --shell /bin/sh --groups i2c,dialout,sudo --password '*' dreamteam"
INHIBIT_PACKAGE_STRIP = "1"

do_install() {
    install -d -m 0700 -o dreamteam -g dreamteam ${D}/home/dreamteam/.ssh
    install -m 0600 -o dreamteam -g dreamteam ${UNPACKDIR}/authorized_keys ${D}/home/dreamteam/.ssh/authorized_keys

    install -d ${D}${sysconfdir}/wpa_supplicant
    install -m 0600 ${UNPACKDIR}/wpa_supplicant-wlan0.conf ${D}${sysconfdir}/wpa_supplicant/wpa_supplicant-wlan0.conf

    install -d ${D}${sysconfdir}/systemd/network
    install -m 0644 ${UNPACKDIR}/20-wlan0.network ${D}${sysconfdir}/systemd/network/

    install -d ${D}${sysconfdir}/sudoers.d
    install -m 0440 ${UNPACKDIR}/90-cse-sudo ${D}${sysconfdir}/sudoers.d/

    # wpa_supplicant@wlan0 brings the interface up before networkd runs DHCP on it
    install -d ${D}${sysconfdir}/systemd/system/multi-user.target.wants
    ln -sf ${systemd_system_unitdir}/wpa_supplicant@.service \
        ${D}${sysconfdir}/systemd/system/multi-user.target.wants/wpa_supplicant@wlan0.service
}

FILES:${PN} = "/home/dreamteam ${sysconfdir}"
RDEPENDS:${PN} = "wpa-supplicant sudo"
