<p align="center"><img src="assets/banner.svg" alt="BIBI Brightness" width="100%"></p>

# BIBI Brightness

**Change brightness of your displays with a simple cross-platform UI in a single binary.**

Cross-Platform tool to change monitor brightness over DDC/CI. Talks to the monitors directly
(no ddcutil needed). DDC/CI must be enabled in the monitor's menu.

## Download

Prebuilt binaries for Linux and Windows are attached to each
[release](https://github.com/matz0r/bibi-brightness/releases).

## Windows

Unzip `bibi-brightness-<version>-windows-x86_64.zip` and start
`brightness.exe`. No installer or drivers needed. The binary is not signed, so
SmartScreen may warn on first start ("More info" → "Run anyway").

## Linux

Needs glibc 2.35 (Ubuntu 22.04) or newer.

On first start, click **Set up access** and enter your password. This is
needed once per PC so the app can talk to the monitors.

<details>
<summary>Manual setup, if the button doesn't work</summary>

```bash
sudo usermod -aG i2c $USER                                   # access to /dev/i2c-*
echo i2c-dev | sudo tee /etc/modules-load.d/i2c-dev.conf     # load module at boot
sudo modprobe i2c-dev                                        # load it now
# then log out and back in
```

</details>

Unpack `bibi-brightness-<version>-linux-x86_64.tar.gz` and, from inside the
unpacked folder, install it into the menu:

```bash
mkdir -p ~/.local/bin && cp brightness ~/.local/bin/
cp brightness.desktop ~/.local/share/applications/
mkdir -p ~/.local/share/icons/hicolor/256x256/apps
cp assets/icon.png ~/.local/share/icons/hicolor/256x256/apps/bibi-brightness.png
```

## Usage

- Each monitor has its own slider.
- Tick **Link** on two or more monitors and their sliders move together.
  A monitor joining the group takes the group's current brightness.
- Links are remembered per monitor model/serial.
- **Presets** (e.g. 60 / 25) set linked monitors to the first value and all
  other monitors to the second.
- Closing the window keeps Brightness running as an icon in the panel
  (Linux) or the notification area next to the clock (Windows). Click the
  icon to show or hide the sliders; right-click it for **Quit**.
- **Autostart** starts Brightness in the tray when you log in.
  Click it again to turn it off.

## Development

Tray icon, building from source, changing presets and releasing: see
[DEVELOPMENT.md](https://github.com/matz0r/bibi-brightness/blob/main/DEVELOPMENT.md).

Third-party license notices are in `THIRD-PARTY-LICENSES.html`, included with
every release.
