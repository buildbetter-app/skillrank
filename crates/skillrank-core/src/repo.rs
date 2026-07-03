//! Repo root + skill-surface resolution.

use crate::skills;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Return the git top-level for cwd, falling back to cwd when not in a git repo.
pub fn repo_root(cwd: &str) -> PathBuf {
    let cwd = if cwd.trim().is_empty() {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        cwd.to_string()
    };
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&cwd)
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !root.is_empty() {
                return PathBuf::from(root);
            }
        }
    }
    PathBuf::from(cwd)
}

/// Choose the skill surface directory for install. An explicit override wins;
/// otherwise the first existing SupportedDirectories entry; otherwise
/// `.claude/skills` is created.
///
/// Returns (relative_path, absolute_path).
pub fn resolve_surface(repo_root: &Path, override_dir: &str) -> (String, PathBuf) {
    let o = override_dir.trim();
    if !o.is_empty() {
        let rel = o.replace('\\', "/");
        let abs = repo_root.join(&rel);
        return (rel, abs);
    }
    if let Ok(discovery) = skills::discover(repo_root) {
        if let Some(surface) = discovery.surface {
            return (
                surface.relative_path.clone(),
                PathBuf::from(surface.absolute_path),
            );
        }
    }
    let rel = ".claude/skills".to_string();
    let abs = repo_root.join(&rel);
    (rel, abs)
}
