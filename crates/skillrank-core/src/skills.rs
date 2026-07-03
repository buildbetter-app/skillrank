//! Discovers skill-surface directories in a repo, mirroring the conventional
//! locations agents read from, so installs land where Claude Code, Codex, and
//! others already look.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// Skill-surface locations, in priority order.
pub const SUPPORTED_DIRECTORIES: &[&str] = &[
    ".agents/skills",
    ".claude/skills",
    ".codex/skills",
    ".agent/skills",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Surface {
    pub relative_path: String,
    pub absolute_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    pub relative_path: String,
    pub absolute_path: String,
    pub surface_relative_path: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryResult {
    pub supported_directories: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<Surface>,
    pub skills: Vec<Skill>,
}

/// Scan repo_root for skill surfaces and the skills within them.
pub fn discover(repo_root: &Path) -> std::io::Result<DiscoveryResult> {
    let mut result = DiscoveryResult {
        supported_directories: SUPPORTED_DIRECTORIES.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    for relative in SUPPORTED_DIRECTORIES {
        let absolute = repo_root.join(relative);
        match std::fs::metadata(&absolute) {
            Ok(m) if m.is_dir() => {
                let surface = Surface {
                    relative_path: relative.to_string(),
                    absolute_path: absolute.to_string_lossy().to_string(),
                };
                if result.surface.is_none() {
                    result.surface = Some(surface.clone());
                }
                result.skills.extend(list_skills_in_surface(&surface)?);
            }
            _ => {}
        }
    }
    Ok(result)
}

fn list_skills_in_surface(surface: &Surface) -> std::io::Result<Vec<Skill>> {
    let mut result = Vec::new();
    let dir = PathBuf::from(&surface.absolute_path);
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let mut skill_path: Option<PathBuf> = None;
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            let candidate = entry.path().join("SKILL.md");
            if candidate.exists() {
                skill_path = Some(candidate);
            }
        } else if name.eq_ignore_ascii_case("SKILL.md")
            || name.to_lowercase().ends_with(".md")
        {
            skill_path = Some(entry.path());
        }
        let Some(skill_path) = skill_path else { continue };

        let rel_within = skill_path
            .strip_prefix(&dir)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| skill_path.to_string_lossy().to_string());
        let repo_relative = format!("{}/{}", surface.relative_path, rel_within.replace('\\', "/"));
        let content = std::fs::read_to_string(&skill_path).unwrap_or_default();
        let mut skill_name = parse_manifest_name(&content);
        if skill_name.trim().is_empty() {
            skill_name = fallback_skill_name(&skill_path);
        }
        result.push(Skill {
            name: skill_name,
            relative_path: repo_relative,
            absolute_path: skill_path.to_string_lossy().to_string(),
            surface_relative_path: surface.relative_path.clone(),
        });
    }
    Ok(result)
}

/// Read the `name:` field from SKILL.md YAML frontmatter.
pub fn parse_manifest_name(content: &str) -> String {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut lines = content.split('\n');
    match lines.next() {
        Some(first) if first.trim() == "---" => {}
        _ => return String::new(),
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("name:") {
            return rest.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
        }
    }
    String::new()
}

fn fallback_skill_name(path: &Path) -> String {
    let base = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if base.eq_ignore_ascii_case("SKILL.md") {
        if let Some(parent) = path.parent().and_then(|p| p.file_name()) {
            let parent = parent.to_string_lossy().to_string();
            if parent != "." && !parent.is_empty() {
                return parent;
            }
        }
    }
    match base.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => base,
    }
}
