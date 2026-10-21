# Development

## Build (Linux)

```bash
# once: Rust toolchain + build dependencies
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
sudo apt install build-essential pkg-config libudev-dev

cargo build --release
# -> target/release/brightness
```

The binary needs the glibc of the build machine or newer. Install it with the
commands from the [README](README.md#linux), using `target/release/brightness`
as the binary.

## i2c access (Linux)

The **Set up access** button (`src/setup.rs`) runs a script as root through
`pkexec`. It loads `i2c-dev` now and at boot
(`/etc/modules-load.d/i2c-dev.conf`), and installs
`/etc/udev/rules.d/60-bibi-brightness-i2c.rules`. That rule tags the GPU's i2c
buses with `uaccess`, so the user logged in at the seat gets access right away,
with no group change and no new login. Other i2c buses stay root only. The
button only appears when `/dev/i2c-*` is missing or not accessible.

## Presets

Edit `PRESETS` at the top of `src/main.rs` to change the preset buttons.

## Releasing

Push a version tag, e.g. `git tag v0.1.0 && git push origin v0.1.0`. GitHub
Actions (`.github/workflows/release.yml`) builds Linux and Windows binaries,
generates the license notices with
[cargo-about](https://github.com/EmbarkStudios/cargo-about) (`about.toml`,
`about.hbs`) and attaches both to the release. Running the workflow manually
only builds the packages as workflow artifacts.
