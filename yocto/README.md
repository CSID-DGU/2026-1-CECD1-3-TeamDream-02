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
| Not in git (on purpose) | `provision/` (SSH public keys, Wi-Fi passwords, upload tokens), `layers/`, `buildtools/`, `build/`, caches |

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
cp -r provision.example provision        # then edit: authorized_keys, wpa_supplicant-wlan0.conf, upload-token-*
. ./env.sh
bitbake cse-image-radon cse-image-servo
```

Images land in `build/tmp/deploy/images/raspberrypi0-wifi/<image>-raspberrypi0-wifi.rootfs.wic.bz2`.
Flash with `bmaptool copy <file>.wic.bz2 /dev/sdX` (writes ~210 MB of the 8.7 GB image; `dd` would write all of it).
Cards must be >= 9 GB. First build takes hours; `downloads/` and `sstate-cache/`
(shared, next to `build/`) make later builds fast.

## What is in the image

- Hardware as on the Raspbian boards: I2C on (`i2c-dev`, group `i2c`), hardware PWM overlay `pwm-2chan`
  (GPIO12/13), UART on without a console (`/dev/serial0` -> `ttyS0`, group `dialout`) for the ZD100; `serial-getty@ttyS0` is
  masked, systemd would otherwise start a login prompt on the sensor line.
- Partitions: boot (vfat, 130 MiB) and a fixed 8 GiB ext4 rootfs (`meta-cse/files/wic/cse-sdimage.wks`); the logs
  live in `/var/lib` on it. The rest of the card is left free. Hostname `radon` / `servo` (also the mDNS name).
- Only the packages the boards need (about 150) and 11 kernel modules (Wi-Fi, I2C, PWM); no Bluetooth, audio,
  modem, NFC, NFS, X11 or locales.
- App OTA, the same updater and signed releases as the Raspbian boards: `cse-ota.timer` (5 min after boot, then
  hourly, up to 10 min random delay) runs `/usr/bin/ota` as user `cse`. Slots live in `/var/lib/cse-ota/slots/{a,b}`,
  `current` points at the active one, config in `/etc/cse/ota.conf`. `cse-ota-seed.service` fills slot a with the
  binary shipped in the image on first boot, and again whenever the active slot is empty (a power cut right after
  the first boot can leave 0-byte files). `CSE_ROOT=/var/lib/cse-ota ota status` shows the state.
- `radon-logger` / `servo-logger` systemd services (user `cse`, sandboxed, `Restart=always`), state in
  `/var/lib/cse-radon` or `/var/lib/cse-servo`. Every record has an `event`:
  - radon: `sample` every 60 s.
  - servo (since fw-v0.2.0): `sample` every 60 s, `led_change` when an indicator light switches (cells read
    every second, three readings must agree), `servo` for every button press.
- Servo board, pressing a button: `sudo servo-press 1` (or `2`; `sudo servo-press 1 1540` to try a position).
  The running logger does the motion, so readings and motion never overlap: one servo at a time, rest -> press ->
  hold -> rest in steps of 20 us / 60 ms, signal off afterwards, range fixed to 1300..1700 us. Positions, hold time
  and the light thresholds are in `/etc/cse/servo.conf` (restart `servo-logger` after editing). Nothing presses
  by itself. The unit switches PWM off before start and after any exit.
- `radon-upload` / `servo-upload` services (same binary, `upload`): send the stored records to the ingest server
  (`server/`, URL `CSE_UPLOAD_URL` in `cse-firmware.inc`, on the board `/etc/cse/upload.conf`) every 5 minutes,
  in order, HTTPS only. The position is kept in `upload.json` in the state dir and moves only after the server
  confirmed a batch, so power cuts and a flaky uplink cost nothing but a resend; records stay on the card.
  Token: `provision/upload-token-<board>` (`openssl rand -hex 32`; its sha256 goes into the server's tokens file),
  installed as `/etc/cse/upload.token`, readable by user `cse` only. In the image since fw-v0.1.4.
- Wi-Fi via wpa_supplicant + systemd-networkd, time via systemd-timesyncd (no RTC on the board): asks
  time.google.com / time.cloudflare.com as soon as the network is up, retries every 10 s. The logger never waits
  for it; records before the sync are marked `ntp_synced:false` and dated by the server from `uptime_s`.
- Login: user `dreamteam` (may read the journal without sudo), SSH key only (password field `*`: no password matches, but the account is not locked;
  a locked account makes sshd refuse keys too), passwordless sudo. Root is locked.
- Persistent journal on the SD card (`/var/log` is a real directory, not tmpfs): capped at 512 MiB, keeps 1 GiB
  free, synced every 30 s (`recipes-core/systemd/files/50-cse-journal.conf`).

## Not done yet

- OS OTA: the app OTA above updates only the logger binaries. Kernel and packages need a new image; the plan is RAUC
  (meta-rauc is pinned but not enabled): A/B rootfs partitions, bundle published as a signed release.
- LTE (ELR22 is a Wi-Fi router, so nothing extra is needed unless it turns out to expose USB data).
- Tested on both boards (2026-11-10): Wi-Fi, SSH, sensors, ZD100, PWM off, slot seeding, both servos, automatic
  OTA fw-v0.1.2 -> fw-v0.1.3 (slot a -> b, health check passed), persistent journal, uploader, timesyncd settings,
  uplink through the LTE modem's Wi-Fi (30 min, both boards, no lost ping), `servo` events with a development
  build. Not yet: a `led_change` from a real light, a press under load, a cold boot with only the LTE uplink.
