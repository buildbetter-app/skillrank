//! Registry base URL and local paths. Does NOT depend on any BuildBetter config —
//! the tool works on its own.

use std::path::PathBuf;

/// The hosted SkillRank registry. Override with SKILLRANK_API_URL for self-hosted
/// registries or local development (e.g. `skillrank serve`).
pub const DEFAULT_API_BASE_URL: &str = "https://api.skillrank.dev";

/// Registry base URL, honoring SKILLRANK_API_URL.
pub fn configured_api_base_url() -> String {
    match std::env::var("SKILLRANK_API_URL") {
        Ok(v) if !v.trim().is_empty() => v.trim().trim_end_matches('/').to_string(),
        _ => DEFAULT_API_BASE_URL.to_string(),
    }
}

/// The skillrank config directory (~/.skillrank), created on demand.
pub fn home() -> std::io::Result<PathBuf> {
    let base = match std::env::var("SKILLRANK_HOME") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v),
        _ => {
            let home = home_dir().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "could not determine home dir")
            })?;
            home.join(".skillrank")
        }
    };
    std::fs::create_dir_all(&base)?;
    Ok(base)
}

/// Where the (optional) registry token is stored.
pub fn auth_path() -> std::io::Result<PathBuf> {
    Ok(home()?.join("auth.json"))
}

/// Cross-platform home directory without an external dependency.
pub fn home_dir() -> Option<PathBuf> {
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return Some(PathBuf::from(h));
        }
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        if !h.is_empty() {
            return Some(PathBuf::from(h));
        }
    }
    None
}
