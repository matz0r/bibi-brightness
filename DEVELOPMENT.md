# Development

## Tray icon

`brightness` opens the window and puts an icon in the panel (Linux) or the
notification area (Windows); closing the window keeps it running there.
`brightness --tray` starts with the icon only. Click the icon to show or hide
the sliders; right-click for Open / Quit. Starting `brightness` again while it
runs just opens the window. The window is a child process (`--window`), so
closing it ends that process (`src/tray/`).

Linux needs StatusNotifierItem support (KDE, Cinnamon, XFCE, MATE, LXQt,
Budgie, Waybar; GNOME only with the AppIndicator extension). Without it, it is
kept alive waiting for the tray. After 15 seconds without a tray it opens a
window with a warning; closing that window is prevented until the tray returns,
and **Quit** explicitly ends it. With no session D-Bus at all, startup falls
back to a standalone window and reports the error. A second start finds the
running tray through a socket in `$XDG_RUNTIME_DIR`; on Windows through a named event.
Windows may put a new icon in the overflow (^) first; drag it onto the taskbar
to keep it visible.

The **Autostart** button (`src/autostart/`) starts it hidden on login, running
the current binary with `--tray`. On Linux it writes
`~/.config/autostart/brightness-tray.desktop`; on Windows it sets the value
`BIBI Brightness` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
Clicking it again removes it. If you move the binary, toggle it off and on to
update the path. The button checks the actual target executable and `--tray`
argument rather than merely checking that an entry exists. Linux also honors
`Hidden=true` and `X-GNOME-Autostart-enabled=false`. The UI refreshes this state
every two seconds.

The **Install** button (`src/install/`) copies the running exe on Windows
to `%LOCALAPPDATA%\Programs\BIBI Brightness\brightness.exe` (an older copy
that is still running is renamed to a temporary backup first), creates
`BIBI Brightness.lnk` in the user's Start menu through PowerShell's
`WScript.Shell`, and repoints the autostart value if it is on. It then ends the
tray of the running copy through the named event `Local\bibi-brightness-quit`
and starts the installed exe. The button is hidden when running from the
installed copy. To uninstall, quit it, turn Autostart off first, then delete
that folder and the shortcut.

On Linux it installs to `~/.local/bin/brightness`, writes the application menu
entry and icon below `$XDG_DATA_HOME` (default `~/.local/share`), and migrates
active autostart to that path. The executable is copied to a temporary file and
renamed so an already running binary can be updated. Version metadata is stored
beside the executable in `brightness.version`. Both platforms compare the
installed version and binary contents, so newer versions and rebuilt binaries
can be installed without offering a downgrade. Missing menu integration can be
repaired by installing again.

The installer waits for the old tray to acknowledge shutdown, starts the new
binary, and waits for its readiness acknowledgement before closing the old UI.
Errors keep the current window open. The old Linux v0.6.0 tray must be quit
manually once because it has no replacement command.

`brightness --install` performs the same installation from the command line;
add `--tray` to restart hidden. `brightness --version` prints the package version
without starting the tray or scanning displays.

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
`about.hbs`) and attaches both to the release. Without git: Actions → Release → Run
workflow on `main` with a tag such as `v0.5.0`; it creates the tag and the
release the same way. Running it with the tag left empty only builds the
packages as workflow artifacts.

## Regression checks

```bash
cargo test --locked
cargo build --locked
# Linux: requires Python gi/Gio and dbus-run-session; uses a temporary home/bus
# and does not change the real desktop, installed app, or autostart entry.
dbus-run-session -- python3 tests/tray_startup.py target/debug/brightness
cargo check --locked --target x86_64-pc-windows-gnu
```

The lifecycle test covers delayed tray startup, concurrent launches, window
close/reopen, panel restart, acknowledged shutdown, installation, replacing a running binary, and
migration of an existing autostart entry.
