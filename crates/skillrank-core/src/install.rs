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
    /// When true (the default), install into a single-level `skillrank-<name>`
    /// directory and prefix the skill's display name, so it's clearly a
    /// SkillRank install and is actually discovered by the agent (which only
    /// scans one directory level deep). When false, use the raw slug path.
    pub prefix: bool,
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
            return Err(format!(
                "skill {:?} is unavailable: {reason}",
                resolved.slug
            ));
        }

        // The slug is registry-controlled and is joined onto the local skill
        // surface below, so reject anything that could escape it (traversal,
        // absolute paths, backslashes) before it touches the filesystem.
        if !is_safe_slug(&resolved.slug) {
            return Err(format!(
                "registry returned an unsafe skill slug {:?}; refusing to install",
                resolved.slug
            ));
        }

        let content = self.fetch_skill_content(&resolved)?;
        let got = compute_content_hash(&content);
        if !hashes_equal(&got, &resolved.content_hash) {
            return Err(format!(
                "content hash mismatch for {}: registry advertised {} but downloaded content hashes to {got}; refusing to install",
                resolved.slug, resolved.content_hash
            ));
        }

        let (surface_rel, surface_abs) = resolve_surface(&opts.repo_root, &opts.surface_override);

        // Decide the on-disk directory + the exact bytes to write. By default we
        // install into a single-level `skillrank-<name>` directory (discoverable +
        // clearly a SkillRank install) and rewrite the display name to match. The
        // registry hash (`resolved.content_hash`) is preserved for update checks;
        // `local_hash` tracks whatever actually lands on disk.
        let (dir_name, write_content) = if opts.prefix {
            let base = skill_base_name(&content, &resolved.slug);
            let final_name =
                choose_prefixed_dir(&opts.repo_root, &surface_rel, &base, &resolved.slug);
            let transformed = rewrite_frontmatter_name(&content, &final_name);
            (final_name, transformed)
        } else {
            (resolved.slug.clone(), content.clone())
        };

        let skill_dir = surface_abs.join(&dir_name);
        let skill_file = skill_dir.join("SKILL.md");
        let skill_path_rel = format!("{surface_rel}/{dir_name}/SKILL.md");
        let local_hash = compute_content_hash(&write_content);

        // Idempotence: the exact (already-transformed) bytes are present -> record
        // the lock and skip the write.
        if let Ok(existing) = std::fs::read_to_string(&skill_file) {
            if hashes_equal(&compute_content_hash(&existing), &local_hash) {
                self.record_lock(opts, &resolved, &skill_path_rel, &surface_rel, &local_hash)?;
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
        std::fs::create_dir_all(&skill_dir).map_err(|e| format!("create skill directory: {e}"))?;
        let tmp = skill_dir.join("SKILL.md.tmp");
        std::fs::write(&tmp, write_content.as_bytes())
            .map_err(|e| format!("write skill content: {e}"))?;
        std::fs::rename(&tmp, &skill_file).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("finalize skill install: {e}")
        })?;

        self.record_lock(opts, &resolved, &skill_path_rel, &surface_rel, &local_hash)?;
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
        local_hash: &str,
    ) -> Result<(), String> {
        let mut lf = Lockfile::load(&opts.repo_root).map_err(|e| e.to_string())?;
        let now = opts.now_rfc3339.clone().unwrap_or_else(now_rfc3339);
        let registry_ref = if resolved.version.is_empty() {
            resolved.slug.clone()
        } else {
            format!("{}@{}", resolved.slug, resolved.version)
        };
        // Only record a distinct local hash when the on-disk bytes actually differ
        // from the pristine registry content (i.e. a transform was applied).
        let local_hash = if hashes_equal(local_hash, &resolved.content_hash) {
            String::new()
        } else {
            local_hash.to_string()
        };
        lf.upsert(LockEntry {
            slug: resolved.slug.clone(),
            registry_ref,
            source_type: resolved.source_type.clone(),
            source: resolved.source_url.clone(),
            skill_path: skill_path_rel.to_string(),
            surface: surface_rel.to_string(),
            computed_hash: resolved.content_hash.clone(),
            local_hash,
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
        // Compare against the on-disk hash (`local_hash`) when a transform was
        // applied at install time; otherwise the pristine registry hash.
        let expected = if e.local_hash.is_empty() {
            &e.computed_hash
        } else {
            &e.local_hash
        };
        let state = match std::fs::read_to_string(&abs) {
            Err(_) => "removed upstream".to_string(),
            Ok(content) => {
                if hashes_equal(&compute_content_hash(&content), expected) {
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
    // Remove the per-skill directory when it is a single safe path segment sitting
    // directly under the recorded surface (a dedicated skill dir). Requiring the
    // dir to equal `surface/<name>` stops a tampered lockfile from turning
    // uninstall into an arbitrary directory delete.
    if let Some(dir) = &dir {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let dedicated_dir = !name.contains('/')
            && is_safe_slug(&name)
            && !entry.surface.is_empty()
            && dir == &repo_root.join(&entry.surface).join(&name);
        if dedicated_dir {
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

/// Derive the base skill name (no prefix) for a `skillrank-`-prefixed install:
/// the SKILL.md frontmatter `name:` if present, else the slug's last segment.
/// Sanitized to a `[a-z0-9-]` token, with any existing `skillrank-` prefix
/// stripped so it is never doubled.
fn skill_base_name(content: &str, slug: &str) -> String {
    let raw = frontmatter_name(content)
        .unwrap_or_else(|| slug.rsplit('/').next().unwrap_or(slug).to_string());
    let sanitized = sanitize_name(&raw);
    let base = sanitized
        .strip_prefix("skillrank-")
        .unwrap_or(&sanitized)
        .to_string();
    if base.is_empty() {
        "skill".to_string()
    } else {
        base
    }
}

/// Choose the final `skillrank-…` directory name, disambiguating with the owner
/// when a *different* skill already occupies `skillrank-<base>` in this repo.
fn choose_prefixed_dir(
    repo_root: &std::path::Path,
    surface_rel: &str,
    base: &str,
    slug: &str,
) -> String {
    let primary = format!("skillrank-{base}");
    let primary_path = format!("{surface_rel}/{primary}/SKILL.md");
    let taken_by_other = Lockfile::load(repo_root)
        .map(|lf| {
            lf.skills
                .iter()
                .any(|e| e.skill_path == primary_path && !e.slug.eq_ignore_ascii_case(slug))
        })
        .unwrap_or(false);
    if taken_by_other {
        let owner = slug
            .split('/')
            .next()
            .map(sanitize_name)
            .unwrap_or_default();
        if !owner.is_empty() {
            return format!("skillrank-{owner}-{base}");
        }
    }
    primary
}

/// Extract the frontmatter `name:` value from a SKILL.md, if present.
fn frontmatter_name(content: &str) -> Option<String> {
    let after = content.strip_prefix("---")?;
    let end = after.find("\n---")?;
    for line in after[..end].split('\n') {
        if let Some(rest) = line.trim_start().strip_prefix("name:") {
            let v = rest.trim().trim_matches(['"', '\'']).trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Rewrite the leading frontmatter `name:` to `new_name`. If there is no
/// frontmatter or no `name:` line, the content is returned unchanged (the
/// directory still carries the prefix, which is the skill's real identity).
fn rewrite_frontmatter_name(content: &str, new_name: &str) -> String {
    let Some(after) = content.strip_prefix("---") else {
        return content.to_string();
    };
    let Some(end) = after.find("\n---") else {
        return content.to_string();
    };
    let (fm, rest) = after.split_at(end); // rest starts with "\n---"
    let mut replaced = false;
    let mut out: Vec<String> = Vec::new();
    for line in fm.split('\n') {
        let trimmed = line.trim_start();
        if !replaced && trimmed.starts_with("name:") {
            let indent = &line[..line.len() - trimmed.len()];
            out.push(format!("{indent}name: {new_name}"));
            replaced = true;
        } else {
            out.push(line.to_string());
        }
    }
    if !replaced {
        return content.to_string();
    }
    format!("---{}{}", out.join("\n"), rest)
}

/// Lowercase + collapse to a `[a-z0-9-]` token suitable for a skill directory
/// and display name. Non-alphanumerics become `-`; runs collapse; edges trimmed.
fn sanitize_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for ch in s.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// True when a registry slug is safe to use as a relative filesystem path:
/// one or more `/`-separated segments, each a non-empty run of `[A-Za-z0-9._-]`
/// that is not `.` or `..`. Rejects absolute paths, backslashes, empty segments,
/// and traversal — the slug is registry-controlled and gets joined onto the
/// local skill surface, so this is the boundary that keeps writes/removes inside
/// the surface directory.
pub fn is_safe_slug(slug: &str) -> bool {
    if slug.is_empty() || slug.len() > 255 || slug.starts_with('/') || slug.contains('\\') {
        return false;
    }
    slug.split('/').all(|seg| {
        !seg.is_empty()
            && seg != "."
            && seg != ".."
            && seg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    })
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

#[cfg(test)]
mod tests {
    use super::{
        frontmatter_name, is_safe_slug, rewrite_frontmatter_name, sanitize_name, skill_base_name,
    };

    #[test]
    fn accepts_normal_slugs() {
        assert!(is_safe_slug("playwright"));
        assert!(is_safe_slug("owner/skill"));
        assert!(is_safe_slug("owner/skill.name_v2-1"));
    }

    #[test]
    fn rewrites_frontmatter_name_preserving_body() {
        let src = "---\nname: brainstorming\ndescription: Do X\n---\n# Body\ntext\n";
        let out = rewrite_frontmatter_name(src, "skillrank-brainstorming");
        assert!(out.contains("name: skillrank-brainstorming"));
        assert!(!out.contains("name: brainstorming\n"));
        assert!(out.contains("description: Do X"));
        assert!(out.ends_with("# Body\ntext\n"));
    }

    #[test]
    fn rewrite_is_noop_without_name_or_frontmatter() {
        let no_name = "---\ndescription: only\n---\nbody";
        assert_eq!(rewrite_frontmatter_name(no_name, "skillrank-x"), no_name);
        let no_fm = "# just a heading\nno frontmatter";
        assert_eq!(rewrite_frontmatter_name(no_fm, "skillrank-x"), no_fm);
    }

    #[test]
    fn extracts_frontmatter_name() {
        assert_eq!(
            frontmatter_name("---\nname: \"my-skill\"\n---\nb"),
            Some("my-skill".to_string())
        );
        assert_eq!(frontmatter_name("no frontmatter"), None);
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(
            sanitize_name("Test Driven_Development!"),
            "test-driven-development"
        );
        assert_eq!(sanitize_name("  --Weird__Name.. "), "weird-name");
    }

    #[test]
    fn base_name_from_frontmatter_and_strips_existing_prefix() {
        let src = "---\nname: skillrank-foo\n---\nb";
        assert_eq!(skill_base_name(src, "owner/foo"), "foo");
        // falls back to slug's last segment when no frontmatter name
        assert_eq!(skill_base_name("no fm", "owner/bar-baz"), "bar-baz");
    }

    #[test]
    fn rejects_traversal_and_absolute() {
        assert!(!is_safe_slug(""));
        assert!(!is_safe_slug("../etc/passwd"));
        assert!(!is_safe_slug("owner/../../etc"));
        assert!(!is_safe_slug("/etc/passwd"));
        assert!(!is_safe_slug("owner/.."));
        assert!(!is_safe_slug(".."));
        assert!(!is_safe_slug("."));
        assert!(!is_safe_slug("owner//skill"));
        assert!(!is_safe_slug("owner\\skill"));
        assert!(!is_safe_slug("owner/skill space"));
    }
}
