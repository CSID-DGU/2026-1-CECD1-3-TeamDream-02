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

# no locale packages (logs and services run in C/English)
IMAGE_LINGUAS = ""

# no root password anywhere; the only way in is the dreamteam SSH key
IMAGE_CLASSES += "extrausers"
EXTRA_USERS_PARAMS = "usermod -L root;"
