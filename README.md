# Brightness

Sliders for monitor brightness over DDC/CI. Talks to the monitors directly
(no ddcutil needed).

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
```

## Usage

- Each monitor has its own slider.
- Tick **Link** on two or more monitors and their sliders move together.
  A monitor joining the group takes the group's current brightness.
- Links are remembered per monitor model/serial.
- **Presets** (e.g. 60 / 25) set linked monitors to the first value and all
  other monitors to the second. Edit `PRESETS` at the top of `src/main.rs`
  to change them.
