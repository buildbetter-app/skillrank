//! Resolve → verify → write → lockfile. Never executes skill content.

use crate::client::{Client, ClientError};
use crate::hash::{compute_content_hash, hashes_equal};
use crate::lockfile::{LockEntry, Lockfile};
use crate::repo::resolve_surface;
use crate::types::{ResolveResponse, ScanTier};
use serde::Serialize;
use std::path::PathBuf;

/// Parameters for [`Client::install`].
#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub reference: String,
    pub repo_root: PathBuf,
    pub surface_override: String,
    /// Injected for deterministic tests; None uses the current time.
    pub now_rfc3339: Option<String>,
}

/// What an install did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub slug: String,
    pub version: String,
    pub skill_path: String,
    pub surface: String,
    pub scan_tier: ScanTier,
    pub content_hash: String,
    pub already_exact: bool,
}

/// One row in `list`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSkill {
    pub slug: String,
    pub skill_path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub version: String,
    pub state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub surface: String,
}

impl Client {
    /// Resolve, verify, and write a skill into the repo surface, updating the
    /// lockfile. Never executes skill content. The caller handles any
    /// confirmation prompt for an unsafe scan tier.
    pub fn install(&self, opts: &InstallOptions) -> Result<InstallResult, String> {
        let resolved = match self.resolve(&opts.reference) {
            Ok(r) => r,
            Err(ClientError::NotFound) => {
                return Err(format!(
                    "skill {:?} not found in the registry",
                    opts.reference
                ))
            }
            Err(e) => return Err(e.to_string()),
        };
        if resolved.tombstoned {
            let reason = if resolved.tombstone_reason.is_empty() {
                "removed upstream".to_string()
            } else {
                resolved.tombstone_reason.clone()
            };
            return Err(format!("skill {:?} is unavailable: {reason}", resolved.slug));
        }

        let content = self.fetch_skill_content(&resolved)?;
        let got = compute_content_hash(&content);
        if !hashes_equal(&got, &resolved.content_hash) {
            return Err(format!(
                "content hash mismatch for {}: registry advertised {} but downloaded content hashes to {got}; refusing to install",
                resolved.slug, resolved.content_hash
            ));
        }

        let (surface_rel, surface_abs) =
            resolve_surface(&opts.repo_root, &opts.surface_override);
        let skill_dir = surface_abs.join(&resolved.slug);
        let skill_file = skill_dir.join("SKILL.md");
        let skill_path_rel = format!("{surface_rel}/{}/SKILL.md", resolved.slug);

        // Idempotence: exact content already present -> record lock, skip write.
        if let Ok(existing) = std::fs::read_to_string(&skill_file) {
            if hashes_equal(&compute_content_hash(&existing), &resolved.content_hash) {
                self.record_lock(opts, &resolved, &skill_path_rel, &surface_rel)?;
                return Ok(InstallResult {
                    slug: resolved.slug.clone(),
                    version: resolved.version.clone(),
                    skill_path: skill_path_rel,
                    surface: surface_rel,
                    scan_tier: resolved.scan_tier,
                    content_hash: resolved.content_hash,
                    already_exact: true,
                });
            }
        }

        // Atomic write: temp then rename, so a failed write leaves no partial install.
        std::fs::create_dir_all(&skill_dir)
            .map_err(|e| format!("create skill directory: {e}"))?;
        let tmp = skill_dir.join("SKILL.md.tmp");
        std::fs::write(&tmp, content.as_bytes()).map_err(|e| format!("write skill content: {e}"))?;
        std::fs::rename(&tmp, &skill_file).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("finalize skill install: {e}")
        })?;

        self.record_lock(opts, &resolved, &skill_path_rel, &surface_rel)?;
        Ok(InstallResult {
            slug: resolved.slug.clone(),
            version: resolved.version.clone(),
            skill_path: skill_path_rel,
            surface: surface_rel,
            scan_tier: resolved.scan_tier,
            content_hash: resolved.content_hash,
            already_exact: false,
        })
    }

    fn fetch_skill_content(&self, resolved: &ResolveResponse) -> Result<String, String> {
        if !resolved.inline_content.trim().is_empty() {
            return Ok(resolved.inline_content.clone());
        }
        if !resolved.raw_content_url.trim().is_empty() {
            return self
                .fetch_raw_content(&resolved.raw_content_url)
                .map_err(|e| e.to_string());
        }
        Err(format!(
            "registry did not provide installable content for {}",
            resolved.slug
        ))
    }

    fn record_lock(
        &self,
        opts: &InstallOptions,
        resolved: &ResolveResponse,
        skill_path_rel: &str,
        surface_rel: &str,
    ) -> Result<(), String> {
        let mut lf = Lockfile::load(&opts.repo_root).map_err(|e| e.to_string())?;
        let now = opts
            .now_rfc3339
            .clone()
            .unwrap_or_else(now_rfc3339);
        let registry_ref = if resolved.version.is_empty() {
            resolved.slug.clone()
        } else {
            format!("{}@{}", resolved.slug, resolved.version)
        };
        lf.upsert(LockEntry {
            slug: resolved.slug.clone(),
            registry_ref,
            source_type: resolved.source_type.clone(),
            source: resolved.source_url.clone(),
            skill_path: skill_path_rel.to_string(),
            surface: surface_rel.to_string(),
            computed_hash: resolved.content_hash.clone(),
            pinned_commit: resolved.pinned_commit.clone(),
            installed_at: now,
            ..Default::default()
        });
        lf.save().map_err(|e| e.to_string())
    }
}

/// Reconcile the lockfile against on-disk surface content and report drift.
pub fn list_installed(repo_root: &std::path::Path) -> std::io::Result<Vec<InstalledSkill>> {
    let lf = Lockfile::load(repo_root)?;
    let mut rows = Vec::new();
    for e in &lf.skills {
        let abs = repo_root.join(&e.skill_path);
        let state = match std::fs::read_to_string(&abs) {
            Err(_) => "removed upstream".to_string(),
            Ok(content) => {
                if hashes_equal(&compute_content_hash(&content), &e.computed_hash) {
                    "ok".to_string()
                } else {
                    "modified".to_string()
                }
            }
        };
        rows.push(InstalledSkill {
            slug: e.slug.clone(),
            skill_path: e.skill_path.clone(),
            version: e.registry_ref.clone(),
            state,
            surface: e.surface.clone(),
        });
    }
    Ok(rows)
}

/// Remove a skill's files and lockfile entry by slug.
pub fn uninstall(repo_root: &std::path::Path, slug: &str) -> Result<String, String> {
    let mut lf = Lockfile::load(repo_root).map_err(|e| e.to_string())?;
    let entry = lf
        .find_by_slug(slug)
        .ok_or_else(|| format!("skill {slug:?} is not installed (no lockfile entry)"))?
        .clone();
    let abs = repo_root.join(&entry.skill_path);
    let dir = abs.parent().map(|p| p.to_path_buf());
    // Remove the per-skill dir when it looks dedicated; otherwise just the file.
    if let Some(dir) = &dir {
        if dir.file_name().map(|n| n.to_string_lossy() == slug).unwrap_or(false) {
            let _ = std::fs::remove_dir_all(dir);
        } else {
            let _ = std::fs::remove_file(&abs);
        }
    } else {
        let _ = std::fs::remove_file(&abs);
    }
    lf.remove(&entry.skill_path);
    lf.save().map_err(|e| e.to_string())?;
    Ok(entry.skill_path)
}

/// Whether a scan tier is safe to install without an extra confirmation prompt.
pub fn safe_scan_tier(tier: ScanTier) -> bool {
    tier.is_safe()
}

/// RFC3339 UTC timestamp (seconds precision) without an external date dependency.
pub fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hour, min, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Howard Hinnant's civil-from-days algorithm (days since 1970-01-01 -> Y/M/D).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
