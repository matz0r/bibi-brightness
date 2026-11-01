//! Update of the installed copy from GitHub releases. Checking is off until
//! the user turns on the **Updates** toggle; then the window of the installed
//! copy looks up the latest release tag at most once per `CHECK_INTERVAL`.
//! The **Update** button downloads that release, unpacks the binary to a
//! temporary folder and runs it with `--install`, which replaces the installed
//! copy and restarts the tray exactly like the Install button.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const REPO: &str = "https://github.com/matz0r/bibi-brightness";
const TEMP_PREFIX: &str = "bibi-brightness-update-";
/// Release archives are a few MB; this only guards against a broken response.
const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;
/// Minimum time between two automatic checks
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[cfg(target_os = "linux")]
const PLATFORM: &str = "linux-x86_64";
#[cfg(windows)]
const PLATFORM: &str = "windows-x86_64";
#[cfg(target_os = "linux")]
const ARCHIVE: &str = "tar.gz";
#[cfg(windows)]
const ARCHIVE: &str = "zip";
#[cfg(target_os = "linux")]
const BINARY: &str = "brightness";
#[cfg(windows)]
const BINARY: &str = "brightness.exe";

pub enum Msg {
    /// GitHub answered: the latest release, if it is newer than this program
    Checked(Option<semver::Version>),
    /// The update was installed and the new copy has started
    Installed,
    Failed(String),
}

/// Releases are only built for x86_64, and only the installed copy updates
/// itself: a download is updated by downloading again.
pub fn eligible() -> bool {
    cfg!(target_arch = "x86_64") && crate::install::is_installed_copy()
}

/// Seconds since the Unix epoch, for the stored time of the last check
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Whether the last successful check at `last` (Unix seconds) is old enough.
/// A last check in the future (clock changed) also counts as due.
pub fn due(last: u64, now: u64) -> bool {
    now < last || now - last >= CHECK_INTERVAL.as_secs()
}

/// Looks for a newer release in the background. Reports nothing when GitHub
/// can't be reached; offline use is normal and the next start tries again.
pub fn check() -> Receiver<Msg> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        remove_leftovers();
        match latest() {
            Ok(latest) => {
                let _ = tx.send(Msg::Checked(is_newer(&latest).then_some(latest)));
            }
            Err(e) => eprintln!("brightness: update check failed: {e}"),
        }
    });
    rx
}

/// Downloads and installs `version` in the background. On success the
/// running tray (and with it this window) is replaced by the new copy.
pub fn install(version: semver::Version) -> Receiver<Msg> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(match download_and_install(&version) {
            Ok(()) => Msg::Installed,
            Err(e) => Msg::Failed(format!("Update failed: {e}")),
        });
    });
    rx
}

/// `brightness --update`: installs the latest release if it is newer than
/// this program. Returns the installed version, or None when up to date.
pub fn update_now() -> Result<Option<semver::Version>, String> {
    let version = latest()?;
    if !is_newer(&version) {
        return Ok(None);
    }
    download_and_install(&version)?;
    Ok(Some(version))
}

fn agent(timeout: Duration, follow_redirects: bool) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(concat!("bibi-brightness/", env!("CARGO_PKG_VERSION")));
    let config = if follow_redirects {
        config
    } else {
        config.max_redirects(0).max_redirects_will_error(false)
    };
    // Windows: SChannel with the system certificate store
    #[cfg(windows)]
    let config = config.tls_config(
        ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build(),
    );
    config.build().into()
}

/// GitHub redirects /releases/latest to /releases/tag/<tag>. Reading the
/// redirect avoids the rate-limited API and any JSON parsing.
fn latest() -> Result<semver::Version, String> {
    let response = agent(Duration::from_secs(10), false)
        .get(format!("{REPO}/releases/latest"))
        .call()
        .map_err(|e| e.to_string())?;
    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .ok_or("GitHub did not name the latest release")?;
    parse_tag_url(location).ok_or_else(|| format!("unexpected release address {location}"))
}

fn parse_tag_url(url: &str) -> Option<semver::Version> {
    let tag = url.rsplit_once("/releases/tag/")?.1;
    semver::Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()
}

pub fn is_newer(version: &semver::Version) -> bool {
    *version > semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap()
}

fn asset_url(version: &semver::Version) -> String {
    format!("{REPO}/releases/download/v{version}/bibi-brightness-{version}-{PLATFORM}.{ARCHIVE}")
}

fn download_and_install(version: &semver::Version) -> Result<(), String> {
    let mut response = agent(Duration::from_secs(120), true)
        .get(asset_url(version))
        .call()
        .map_err(|e| format!("can't download version {version}: {e}"))?;
    let archive = response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .read_to_vec()
        .map_err(|e| format!("download interrupted: {e}"))?;

    // A fresh folder per attempt; create_dir fails instead of reusing a
    // folder someone else prepared.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let dir = std::env::temp_dir().join(format!("{TEMP_PREFIX}{}-{nonce}", std::process::id()));
    std::fs::create_dir(&dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let result = unpack_and_run(&archive, &dir, version);
    // On success the tray usually ends this window before it gets here; the
    // next update check removes what is left.
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn unpack_and_run(archive: &[u8], dir: &Path, version: &semver::Version) -> Result<(), String> {
    let exe = dir.join(BINARY);
    unpack(archive, &exe)?;

    // Make sure the download is the announced version before it replaces
    // anything.
    let output = Command::new(&exe)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("can't start the downloaded program: {e}"))?;
    let reported = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || reported.trim() != version.to_string() {
        return Err(format!(
            "the download reports version {:?} instead of {version}",
            reported.trim()
        ));
    }

    // Errors go to a file: this window may be ended halfway, when the old
    // tray is replaced, and the installer must not depend on it.
    let log = dir.join("install.log");
    let log_file = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let status = Command::new(&exe)
        .arg("--install")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log_file)
        .status()
        .map_err(|e| format!("can't start the installer: {e}"))?;
    if status.success() {
        return Ok(());
    }
    let message = std::fs::read_to_string(&log).unwrap_or_default();
    let message = message.lines().last().unwrap_or("installation failed");
    Err(message
        .strip_prefix("brightness: ")
        .unwrap_or(message)
        .into())
}

#[cfg(target_os = "linux")]
fn unpack(archive: &[u8], exe: &Path) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries().map_err(|e| format!("broken archive: {e}"))? {
        let mut entry = entry.map_err(|e| format!("broken archive: {e}"))?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        if entry.header().entry_type().is_file() && path.file_name() == Some(BINARY.as_ref()) {
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o755)
                .open(exe)
                .map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("broken archive: {e}"))?;
            return Ok(());
        }
    }
    Err(format!("{BINARY} is missing from the download"))
}

#[cfg(windows)]
fn unpack(archive: &[u8], exe: &Path) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
        .map_err(|e| format!("broken archive: {e}"))?;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| format!("broken archive: {e}"))?;
        let name = file
            .name()
            .map_err(|e| format!("broken archive: {e}"))?
            .into_owned();
        if file.is_file() && Path::new(&name).file_name() == Some(BINARY.as_ref()) {
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(exe)
                .map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut out).map_err(|e| format!("broken archive: {e}"))?;
            return Ok(());
        }
    }
    Err(format!("{BINARY} is missing from the download"))
}

/// Folders of earlier updates whose window was ended before it could clean up.
fn remove_leftovers() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let own = format!("{TEMP_PREFIX}{}-", std::process::id());
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(TEMP_PREFIX) && !name.starts_with(&own) {
            // Fails harmlessly for other users' folders
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_redirect() {
        assert_eq!(
            parse_tag_url("https://github.com/matz0r/bibi-brightness/releases/tag/v0.6.3"),
            Some(semver::Version::new(0, 6, 3))
        );
        assert_eq!(
            parse_tag_url("https://github.com/matz0r/bibi-brightness/releases"),
            None
        );
        assert_eq!(
            parse_tag_url("https://github.com/x/releases/tag/nightly"),
            None
        );
    }

    #[test]
    fn names_release_asset() {
        let url = asset_url(&semver::Version::new(1, 2, 3));
        assert!(url.ends_with(&format!(
            "/v1.2.3/bibi-brightness-1.2.3-{PLATFORM}.{ARCHIVE}"
        )));
    }

    #[test]
    fn checks_once_per_interval() {
        let day = CHECK_INTERVAL.as_secs();
        assert!(due(0, 1_000_000));
        assert!(!due(1_000_000, 1_000_000));
        assert!(!due(1_000_000, 1_000_000 + day - 1));
        assert!(due(1_000_000, 1_000_000 + day));
        assert!(due(1_000_000 + 60, 1_000_000));
    }

    #[test]
    fn only_newer_versions_are_updates() {
        let current = semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
        assert!(!is_newer(&current));
        assert!(is_newer(&semver::Version::new(current.major + 1, 0, 0)));
    }

    /// Talks to GitHub: `cargo test -- --ignored`. Installs nothing.
    #[test]
    #[ignore]
    fn downloads_latest_release() {
        let version = latest().unwrap();
        let mut response = agent(Duration::from_secs(120), true)
            .get(asset_url(&version))
            .call()
            .unwrap();
        let archive = response
            .body_mut()
            .with_config()
            .limit(MAX_DOWNLOAD)
            .read_to_vec()
            .unwrap();
        let dir = std::env::temp_dir().join(format!("bibi-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(BINARY);
        unpack(&archive, &exe).unwrap();
        let output = Command::new(&exe).arg("--version").output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            version.to_string()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
