SUMMARY = "CSE capstone board image (${CSE_BOARD}, Pi Zero W)"
LICENSE = "MIT"

inherit core-image

IMAGE_FEATURES += "ssh-server-openssh"

# Explicit list instead of core-image's default (packagegroup-base-extended pulls bluetooth, audio, modem, NFS, X11, ...).
# Kernel modules are listed by name too (see MACHINE_EXTRA_RRECOMMENDS:remove in conf/distro/cse.conf);
# kernel-module packages bring their module dependencies (cfg80211, rfkill, ...) via modules.dep.
IMAGE_INSTALL = " \
    packagegroup-core-boot \
    cse-${CSE_BOARD} \
    cse-provision \
    udev-rules-cse \
    udev-rules-rpi \
    i2c-tools \
    usbutils \
    libusb1 \
    wpa-supplicant \
    iw \
    wireless-regdb-static \
    linux-firmware-rpidistro-bcm43430 \
    kernel-module-brcmfmac \
    kernel-module-brcmfmac-cyw \
    kernel-module-brcmutil \
    kernel-module-cfg80211 \
    kernel-module-i2c-dev \
    kernel-module-i2c-bcm2708 \
    kernel-module-pwm-bcm2835 \
    avahi-daemon \
    avahi-libnss-mdns \
    sudo \
    curl \
    ca-certificates \
    e2fsprogs \
    dosfstools \
    tzdata-core \
    tzdata-asia \
"

# fixed 8 GiB rootfs, see meta-cse/files/wic/cse-sdimage.wks
WKS_FILE = "cse-sdimage.wks"

# no locale packages (logs and services run in C/English)
IMAGE_LINGUAS = ""

# no root password anywhere; the only way in is the dreamteam SSH key
IMAGE_CLASSES += "extrausers"
EXTRA_USERS_PARAMS = "usermod -L root;"

# base-files is shared by both images, so the board name is written into the rootfs here (default would be the MACHINE name);
# it is also the mDNS name: radon.local / servo.local
cse_set_hostname() {
    echo ${CSE_BOARD} > ${IMAGE_ROOTFS}${sysconfdir}/hostname
    sed -i 's/\braspberrypi0-wifi\b/${CSE_BOARD}/g' ${IMAGE_ROOTFS}${sysconfdir}/hosts
}
ROOTFS_POSTPROCESS_COMMAND += "cse_set_hostname"

# ttyS0 (serial0) is the ZD100 line on the radon board. systemd-getty-generator still starts a login getty on it,
# which changes the port settings and reads the sensor's replies, so it is masked on both boards.
cse_mask_serial_getty() {
    ln -sf /dev/null ${IMAGE_ROOTFS}${sysconfdir}/systemd/system/serial-getty@ttyS0.service
}
ROOTFS_POSTPROCESS_COMMAND += "cse_mask_serial_getty"
