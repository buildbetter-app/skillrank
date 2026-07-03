//! Repo-root lockfile recording installed skills. Preserves foreign fields (top
//! level and per entry) so it can be co-owned by other tooling.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// Namespaced to avoid colliding with other ecosystems' lockfiles.
pub const LOCKFILE_NAME: &str = "skill-registry-lock.json";

/// One installed skill. Unknown fields on disk are preserved via `extra`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LockEntry {
    pub slug: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub registry_ref: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    pub skill_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub surface: String,
    pub computed_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pinned_commit: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub installed_at: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The on-disk lockfile document. Foreign top-level keys are preserved in `extra`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile {
    pub version: i32,
    #[serde(default)]
    pub skills: Vec<LockEntry>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
    #[serde(skip)]
    path: PathBuf,
}

impl Lockfile {
    /// The lockfile path for a repo root.
    pub fn path_for(repo_root: &Path) -> PathBuf {
        repo_root.join(LOCKFILE_NAME)
    }

    /// Read the lockfile at repo_root, or return an empty v1 lockfile if none exists.
    pub fn load(repo_root: &Path) -> std::io::Result<Lockfile> {
        let path = Self::path_for(repo_root);
        match std::fs::read_to_string(&path) {
            Ok(data) if !data.trim().is_empty() => {
                let mut lf: Lockfile = serde_json::from_str(&data).map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("parse lockfile {}: {e}", path.display()),
                    )
                })?;
                lf.path = path;
                if lf.version == 0 {
                    lf.version = 1;
                }
                Ok(lf)
            }
            _ => Ok(Lockfile {
                version: 1,
                skills: Vec::new(),
                extra: Map::new(),
                path,
            }),
        }
    }

    /// Insert or replace an entry keyed by skill_path, preserving foreign fields.
    pub fn upsert(&mut self, mut entry: LockEntry) {
        if let Some(existing) = self
            .skills
            .iter_mut()
            .find(|e| e.skill_path == entry.skill_path)
        {
            for (k, v) in existing.extra.clone() {
                entry.extra.entry(k).or_insert(v);
            }
            *existing = entry;
            return;
        }
        self.skills.push(entry);
    }

    /// Delete an entry by skill_path; reports whether one was removed.
    pub fn remove(&mut self, skill_path: &str) -> bool {
        let before = self.skills.len();
        self.skills.retain(|e| e.skill_path != skill_path);
        self.skills.len() != before
    }

    /// Return the first entry matching slug (case-insensitive).
    pub fn find_by_slug(&self, slug: &str) -> Option<&LockEntry> {
        self.skills
            .iter()
            .find(|e| e.slug.eq_ignore_ascii_case(slug))
    }

    /// Write the lockfile back with stable ordering, preserving foreign fields.
    pub fn save(&mut self) -> std::io::Result<()> {
        self.skills.sort_by(|a, b| a.skill_path.cmp(&b.skill_path));
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(&self.path, format!("{json}\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("skillrank-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn round_trip_preserves_foreign_fields() {
        let dir = tmpdir("lf");
        let seed = r#"{"version":1,"toolMeta":{"writtenBy":"other"},"skills":[{"slug":"owner/foreign","skillPath":".claude/skills/foreign/SKILL.md","computedHash":"sha256:aaa","customField":"keep"}]}"#;
        std::fs::write(dir.join(LOCKFILE_NAME), seed).unwrap();

        let mut lf = Lockfile::load(&dir).unwrap();
        lf.upsert(LockEntry {
            slug: "owner/ours".into(),
            skill_path: ".claude/skills/ours/SKILL.md".into(),
            computed_hash: "sha256:bbb".into(),
            source_type: "github".into(),
            ..Default::default()
        });
        lf.save().unwrap();

        let raw = std::fs::read_to_string(dir.join(LOCKFILE_NAME)).unwrap();
        assert!(raw.contains("toolMeta"), "foreign top-level key dropped");
        assert!(raw.contains("customField"), "foreign per-entry key dropped");

        let reloaded = Lockfile::load(&dir).unwrap();
        assert_eq!(reloaded.skills.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remove_reports_correctly() {
        let dir = tmpdir("lf-rm");
        let mut lf = Lockfile::load(&dir).unwrap();
        lf.upsert(LockEntry { slug: "a".into(), skill_path: "p/a".into(), computed_hash: "h".into(), ..Default::default() });
        assert!(lf.remove("p/a"));
        assert!(!lf.remove("p/a"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
