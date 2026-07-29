//! `skillrank update` — replace the current binary with the latest GitHub
//! release asset for this platform.
//!
//! The release lookup ([`latest_release`]) and the download+verify+swap
//! ([`apply`]) are also what the startup update check in [`crate::update_check`]
//! uses, so there is exactly one updater in this binary.

use crate::flags::Flags;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/buildbetter-app/skillrank/releases/latest";
const DOWNLOAD_MIN_BYTES: usize = 100 * 1024;

pub fn run(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let current = env!("CARGO_PKG_VERSION");

    let release = match latest_release(None) {
        Ok(release) => release,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let latest = release.version();

    if !is_newer(latest, current) {
        if f.bool("check") {
            println!("up to date");
        } else {
            println!("skillrank {current} is already up to date.");
        }
        return 0;
    }

    if f.bool("check") {
        println!("update available: {latest} (current {current})");
        return 0;
    }

    let latest = latest.to_string();
    match apply(&release) {
        Ok(()) => {
            println!("Updated skillrank {current} -> {latest}");
            0
        }
        Err(message) => {
            eprintln!("{message}");
            1
        }
    }
}

pub(crate) struct Release {
    tag_name: String,
}

impl Release {
    /// The release version without the tag's `v` prefix.
    pub(crate) fn version(&self) -> &str {
        self.tag_name.trim_start_matches('v')
    }
}

/// Download this platform's asset for `release` and swap it over the running
/// binary. `Err` carries a message that is ready to print as-is.
pub(crate) fn apply(release: &Release) -> Result<(), String> {
    let asset = asset_name(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        format!(
            "error: unsupported platform: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let url = format!(
        "https://github.com/buildbetter-app/skillrank/releases/download/{}/{}",
        release.tag_name, asset
    );
    let bytes = download_asset(&url).map_err(|e| format!("error: {e}"))?;
    if bytes.len() < DOWNLOAD_MIN_BYTES {
        return Err(format!(
            "error: downloaded asset is suspiciously small ({} bytes)",
            bytes.len()
        ));
    }

    let exe = std::env::current_exe()
        .map_err(|e| format!("error: could not resolve current executable: {e}"))?;
    replace_exe(&exe, &bytes).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "cannot replace {}: permission denied. Re-run with sudo, or re-run the installer: curl -fsSL skillrank.dev | sh",
                exe.display()
            )
        } else {
            format!("error: cannot replace {}: {e}", exe.display())
        }
    })
}

/// Look up the latest GitHub release. `timeout` bounds the whole request; the
/// startup check passes one so a dead network can never hang the CLI, while
/// `skillrank update` (which the user is waiting on deliberately) passes none.
pub(crate) fn latest_release(timeout: Option<Duration>) -> Result<Release, String> {
    let mut request = ureq::get(LATEST_RELEASE_URL)
        .set("User-Agent", "skillrank")
        .set("Accept", "application/vnd.github+json");
    if let Some(timeout) = timeout {
        request = request.timeout(timeout);
    }
    let resp = request.call().map_err(http_error)?;
    let value = resp
        .into_json::<Value>()
        .map_err(|e| format!("could not parse GitHub release response: {e}"))?;
    let tag_name = value
        .get("tag_name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "GitHub release response did not include tag_name".to_string())?
        .to_string();
    Ok(Release { tag_name })
}

fn download_asset(url: &str) -> Result<Vec<u8>, String> {
    let resp = ureq::get(url)
        .set("User-Agent", "skillrank")
        .call()
        .map_err(http_error)?;
    let mut reader = resp.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| format!("could not read downloaded asset: {e}"))?;
    Ok(bytes)
}

fn replace_exe(exe: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let tmp = temp_path(dir);
    let write_result = write_executable(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, exe));
    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write_result
}

fn write_executable(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn temp_path(dir: &Path) -> PathBuf {
    dir.join(format!(
        ".skillrank-update-{}-{}.tmp",
        std::process::id(),
        env!("CARGO_PKG_VERSION")
    ))
}

fn http_error(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            if body.trim().is_empty() {
                format!("HTTP {code}")
            } else {
                format!("HTTP {code}: {}", body.trim())
            }
        }
        ureq::Error::Transport(t) => t.to_string(),
    }
}

fn asset_name(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("skillrank-macos-aarch64"),
        ("macos", "x86_64") => Some("skillrank-macos-x64"),
        ("linux", "x86_64") => Some("skillrank-linux-x64"),
        ("linux", "aarch64") => Some("skillrank-linux-aarch64"),
        _ => None,
    }
}

/// Numeric, dot-separated comparison. Non-numeric parts (a `-rc.1` suffix, a
/// malformed tag) parse as 0, so a prerelease sorts above the release it
/// precedes — harmless because GitHub's `releases/latest` never returns a
/// prerelease, and deliberately unchanged so `skillrank update` behaves exactly
/// as it did before the startup check started sharing this function.
pub(crate) fn is_newer(latest: &str, current: &str) -> bool {
    let latest_parts = version_parts(latest);
    let current_parts = version_parts(current);
    let len = latest_parts.len().max(current_parts.len());
    for i in 0..len {
        let latest_part = latest_parts.get(i).copied().unwrap_or(0);
        let current_part = current_parts.get(i).copied().unwrap_or(0);
        if latest_part != current_part {
            return latest_part > current_part;
        }
    }
    false
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_name_maps_supported_platforms() {
        assert_eq!(
            asset_name("macos", "aarch64"),
            Some("skillrank-macos-aarch64")
        );
        assert_eq!(asset_name("macos", "x86_64"), Some("skillrank-macos-x64"));
        assert_eq!(asset_name("linux", "x86_64"), Some("skillrank-linux-x64"));
        assert_eq!(
            asset_name("linux", "aarch64"),
            Some("skillrank-linux-aarch64")
        );
    }

    #[test]
    fn asset_name_rejects_unsupported_platform() {
        assert_eq!(asset_name("windows", "x86_64"), None);
    }

    #[test]
    fn is_newer_compares_numeric_parts() {
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.10.0", "0.9.0"));
    }

    #[test]
    fn is_newer_treats_equal_and_padded_versions_as_not_newer() {
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(!is_newer("1.2.0", "1.2"));
        assert!(!is_newer("", ""));
        assert!(is_newer("1.2.1", "1.2"));
    }

    #[test]
    fn is_newer_never_downgrades_on_malformed_input() {
        // Unparseable parts read as 0, so garbage can never look newer than a
        // real version — the startup check stays quiet instead of nagging.
        assert!(!is_newer("not-a-version", "0.1.4"));
        assert!(!is_newer("", "0.1.4"));
        assert!(!is_newer("v0.1.4", "0.1.4"));
        assert!(is_newer("0.1.4", "not-a-version"));
    }

    #[test]
    fn is_newer_sorts_prereleases_above_their_base_version() {
        // Documented, pre-existing behaviour: `-rc` is not understood, so the
        // suffix becomes a trailing 0 and `.1` an extra segment. It costs
        // nothing in practice because GitHub's releases/latest never returns a
        // prerelease, and the startup check reads exactly that endpoint.
        assert!(is_newer("0.2.0-rc.1", "0.1.4"));
        assert!(is_newer("0.2.0-rc.1", "0.2.0"));
        assert!(!is_newer("0.2.0-rc", "0.2.0"));
        assert!(!is_newer("0.1.9-rc.1", "0.2.0"));
    }

    #[test]
    fn release_version_strips_the_tag_prefix() {
        assert_eq!(
            Release {
                tag_name: "v0.2.0".into()
            }
            .version(),
            "0.2.0"
        );
        assert_eq!(
            Release {
                tag_name: "0.2.0".into()
            }
            .version(),
            "0.2.0"
        );
    }
}
