<p align="center"><img src="assets/banner.svg" alt="BIBI Brightness" width="100%"></p>

# BIBI Brightness

**Change brightness of your displays with a simple UI using a single binary.**

Cross-Platform tool to change monitor brightness over DDC/CI. Talks to the monitors directly
(no ddcutil needed).

## Download

Prebuilt binaries for Linux and Windows (x86_64) are attached to each
[release](https://github.com/matz0r/bibi-brightness/releases), together with
`THIRD-PARTY-LICENSES.html`. The Linux binary needs glibc 2.35 (Ubuntu 22.04)
or newer; see the run requirements below.

## Windows

Unzip `bibi-brightness-<version>-windows-x86_64.zip` and start
`brightness.exe`. No installer or drivers needed; DDC/CI must be enabled in
the monitor's menu. The binary is not signed, so SmartScreen may warn on first
start ("More info" → "Run anyway").

## Build (Linux)

```bash
# once: Rust toolchain + build dependencies
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
sudo apt install build-essential pkg-config libudev-dev

cargo build --release
# -> target/release/brightness
```

## Run requirements (every PC)

```bash
sudo usermod -aG i2c $USER                                   # access to /dev/i2c-*
echo i2c-dev | sudo tee /etc/modules-load.d/i2c-dev.conf     # load module at boot
sudo modprobe i2c-dev                                        # load it now
# then log out and back in
```

Needs glibc of the build machine or newer.

## Install into the menu

```bash
mkdir -p ~/.local/bin && cp target/release/brightness ~/.local/bin/
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
  other monitors to the second. Edit `PRESETS` at the top of `src/main.rs`
  to change them.

## Tray icon (Linux)

`brightness --tray` puts an icon in the panel instead of opening the window.
Click it to show or hide the sliders; right-click for Open / Quit. Works on
desktops with StatusNotifierItem support (KDE, Cinnamon, XFCE, MATE, LXQt,
Budgie, Waybar; GNOME only with the AppIndicator extension). Without one,
`--tray` just opens the window.

Start it on login:

```bash
mkdir -p ~/.config/autostart
sed 's|brightness"|brightness --tray"|' brightness.desktop > ~/.config/autostart/brightness-tray.desktop
```

## Releasing

Push a version tag, e.g. `git tag v0.1.0 && git push origin v0.1.0`. GitHub
Actions (`.github/workflows/release.yml`) builds Linux and Windows binaries,
generates the license notices with
[cargo-about](https://github.com/EmbarkStudios/cargo-about) (`about.toml`,
`about.hbs`) and attaches both to the release. Running the workflow manually
only builds the packages as workflow artifacts.
