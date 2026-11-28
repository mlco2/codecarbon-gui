//! The `afoot` release this version of the app installs.
//!
//! One release is pinned per version of the app: its files' SHA-256 are
//! written here, so what "Install afoot" puts on the computer is exactly what
//! was reviewed with this source, whatever the download server says.
//! Afterwards `afoot self update` (Settings → Updates) moves to newer
//! releases, verifying their Sigstore signatures itself.
//!
//! To pin another release: change [`VERSION`] and copy the four hashes
//! from the `SHA256SUMS` file of that release.

use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The pinned release of <https://github.com/mlco2/agentic-footprint>.
pub const VERSION: &str = "0.1.1";

/// No `afoot` binary is anywhere near this size: more is not the release.
const MAX_BYTES: usize = 64 * 1024 * 1024;

/// One platform's file in the release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin {
    /// The asset's name in the release.
    pub asset: &'static str,
    /// Its SHA-256, lowercase hexadecimal.
    pub sha256: &'static str,
}

const PINS: &[(&str, &str, Pin)] = &[
    (
        "macos",
        "aarch64",
        Pin {
            asset: "afoot-aarch64-apple-darwin",
            sha256: "4a5bc23ac5f9ffb3e4f73dd49c8022b8eed6a5ce5ef29adc749f8d369573002e",
        },
    ),
    (
        "macos",
        "x86_64",
        Pin {
            asset: "afoot-x86_64-apple-darwin",
            sha256: "b2a0fb6dc2aa5d5210ea1b6d0b107f4d55b9a0c2e3710131503474aaa27b0fd7",
        },
    ),
    (
        "linux",
        "x86_64",
        Pin {
            asset: "afoot-x86_64-unknown-linux-gnu",
            sha256: "96ef8d8a04cb10835dcfaade64b8b14091cd68e359618fe908c9faf939d81933",
        },
    ),
    (
        "windows",
        "x86_64",
        Pin {
            asset: "afoot-x86_64-pc-windows-msvc.exe",
            sha256: "c60647b3a0915abe7ba75f1b96ea34ff124849f04bd5dede7101411f091c6909",
        },
    ),
];

/// The release file for an operating system and architecture, named as
/// `std::env::consts::{OS, ARCH}` name them; `None`: no release for it.
pub fn pin_for(os: &str, arch: &str) -> Option<Pin> {
    PINS.iter()
        .find(|(pin_os, pin_arch, _)| *pin_os == os && *pin_arch == arch)
        .map(|(_, _, pin)| *pin)
}

/// Where the pinned release's `asset` is downloaded from.
pub fn url(asset: &str) -> String {
    format!("https://github.com/mlco2/agentic-footprint/releases/download/v{VERSION}/{asset}")
}

/// `bytes` are exactly the pinned file.
pub fn matches(bytes: &[u8], pin: Pin) -> bool {
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    digest == pin.sha256
}

/// The name of the `afoot` executable on this system.
pub fn binary_name() -> &'static str {
    if cfg!(windows) {
        "afoot.exe"
    } else {
        "afoot"
    }
}

/// Download the pinned file: https only, redirects included, and no more
/// than [`MAX_BYTES`].
pub async fn download(pin: Pin) -> Result<Vec<u8>, String> {
    // The same TLS setup as the updater plugin, which may not have run yet.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(std::time::Duration::from_secs(120))
        .user_agent(concat!("codecarbon-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(url(pin.asset))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err(format!("the download is larger than {MAX_BYTES} bytes"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Put `bytes` in `dir` as the executable [`binary_name`], replacing one
/// that is there in a single step. Returns its path.
pub fn place(bytes: &[u8], dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let dest = dir.join(binary_name());
    let tmp = dir.join(format!("{}.tmp", binary_name()));
    let written = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o755);
        }
        let mut file = options.open(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &dest)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_platform_has_a_pinned_file_with_a_real_hash() {
        for (os, arch) in [
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("linux", "x86_64"),
            ("windows", "x86_64"),
        ] {
            let pin = pin_for(os, arch).unwrap_or_else(|| panic!("no pin for {os} {arch}"));
            assert_eq!(pin.sha256.len(), 64, "{}", pin.asset);
            assert!(
                pin.sha256
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "{}: lowercase hexadecimal",
                pin.asset
            );
        }
        assert_eq!(pin_for("linux", "aarch64"), None, "not released");
        assert_eq!(pin_for("freebsd", "x86_64"), None);
    }

    #[test]
    fn the_file_comes_from_the_pinned_release_over_https() {
        assert_eq!(
            url("afoot-x86_64-unknown-linux-gnu"),
            format!(
                "https://github.com/mlco2/agentic-footprint/releases/download/v{VERSION}/afoot-x86_64-unknown-linux-gnu"
            )
        );
    }

    /// The real download, from GitHub: run by hand after pinning a release.
    #[tokio::test]
    #[ignore = "downloads from the network"]
    async fn the_pinned_file_for_this_computer_downloads_and_matches() {
        let pin = pin_for(std::env::consts::OS, std::env::consts::ARCH).expect("a pin");
        let bytes = download(pin).await.expect("the download");
        assert!(matches(&bytes, pin), "{} is not the pinned file", pin.asset);
    }

    #[test]
    fn only_the_pinned_bytes_match() {
        // SHA-256 of "abc".
        let pin = Pin {
            asset: "x",
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };
        assert!(matches(b"abc", pin));
        assert!(!matches(b"abd", pin));
        assert!(!matches(b"", pin));
    }

    #[test]
    fn the_file_is_placed_executable_and_replaces_the_old_one() {
        let dir = std::env::temp_dir().join(format!("cc-afoot-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dest = place(b"first", &dir.join("afoot")).unwrap();
        assert_eq!(dest, dir.join("afoot").join(binary_name()));
        let again = place(b"second", &dir.join("afoot")).unwrap();
        assert_eq!(std::fs::read(&again).unwrap(), b"second");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&again).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        let left: Vec<_> = std::fs::read_dir(dir.join("afoot"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left.len(), 1, "no temporary file is left: {left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
