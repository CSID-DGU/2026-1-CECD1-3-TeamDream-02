# CSE Yocto (Pi Zero W)

Yocto 6.0 **Wrynose** (LTS), `MACHINE=raspberrypi0-wifi` (ARMv6 arm1176jzf-s), systemd, no root password.
Two images, one per board: `cse-image-radon`, `cse-image-servo`.

## Reproducibility

| What | How it is pinned |
|---|---|
| Layers (bitbake, openembedded-core, meta-yocto, meta-openembedded, meta-raspberrypi, meta-rauc) | exact commits in `layers.lock`, fetched by `setup-layers.sh` |
| Host toolchain | Yocto buildtools 6.0.4, URL + sha256 in `fetch-buildtools.sh` |
| Distro/hardware config | `meta-cse/conf/distro/cse.conf` (not `build/conf`) |
| Firmware | `cse-radon` / `cse-servo` recipes install the binaries of the signed GitHub release `fw-v*`, pinned by sha256 |
| Not in git (on purpose) | `provision/` (SSH public keys, Wi-Fi passwords), `layers/`, `buildtools/`, `build/`, caches |

To move to a new firmware release: in `meta-cse/recipes-cse/cse-firmware/cse-firmware.inc` change `CSE_TAG`,
`CSE_BUILD` (the release's `build` in `manifest.json`) and the `ota` sha256; in `cse-radon_*.bb` / `cse-servo_*.bb`
the `bin` sha256 (`sha256sum` of the downloaded `radon` / `servo` / `ota` assets). This only changes what a freshly
flashed card starts with: running boards update themselves (see OTA below).

## First build

```
sudo dnf install diffstat perl perl-Thread-Queue perl-bignum perl-FindBin perl-File-Compare perl-open \
  texinfo chrpath lz4 rpcgen glibc-langpack-en python3-GitPython python3-jinja2 ccache   # Fedora
cd yocto
./setup-layers.sh
./fetch-buildtools.sh
cp -r provision.example provision        # then edit: authorized_keys, wpa_supplicant-wlan0.conf
. ./env.sh
bitbake cse-image-radon cse-image-servo
```

Images land in `build/tmp/deploy/images/raspberrypi0-wifi/<image>-raspberrypi0-wifi.rootfs.wic.bz2`.
Flash with `bmaptool copy <file>.wic.bz2 /dev/sdX`. First build takes hours; `downloads/` and `sstate-cache/`
(shared, next to `build/`) make later builds fast.

## What is in the image

- Hardware as on the Raspbian boards: I2C on (`i2c-dev`, group `i2c`), hardware PWM overlay `pwm-2chan`
  (GPIO12/13), UART on without a console (`/dev/serial0` -> `ttyS0`, group `dialout`) for the ZD100.
- App OTA, the same updater and signed releases as the Raspbian boards: `cse-ota.timer` (5 min after boot, then
  hourly, up to 10 min random delay) runs `/usr/bin/ota` as user `cse`. Slots live in `/var/lib/cse-ota/slots/{a,b}`,
  `current` points at the active one, config in `/etc/cse/ota.conf`. On first boot tmpfiles seeds slot a with the
  binary shipped in the image. `CSE_ROOT=/var/lib/cse-ota ota status` shows the state.
- `radon-logger` / `servo-logger` systemd services (user `cse`, sandboxed, `Restart=always`), same logger and
  record format as the Raspbian setup, state in `/var/lib/cse-radon` or `/var/lib/cse-servo`.
  The servo service switches PWM off before start and after any exit and never moves a servo.
- Wi-Fi via wpa_supplicant + systemd-networkd, time via systemd-timesyncd (no RTC on the board).
- Login: user `dreamteam`, SSH key only, passwordless sudo. Root is locked.

## Not done yet

- OS OTA: the app OTA above updates only the logger binaries. Kernel and packages need a new image; the plan is RAUC
  (meta-rauc is pinned but not enabled): A/B rootfs partitions, bundle published as a signed release.
- LTE (ELR22 is a Wi-Fi router, so nothing extra is needed unless it turns out to expose USB data).
- Everything here is built but only partly tested on real hardware, see the checklist in the PR description.
