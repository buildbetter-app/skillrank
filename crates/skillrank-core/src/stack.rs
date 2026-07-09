//! Repo stack detection used by `recommend` to filter the registry.

use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Default)]
pub struct DetectedStack {
    pub stacks: Vec<String>,
    pub evidence: Vec<String>,
}

struct Probe {
    file: &'static str,
    needle: Option<&'static str>,
    stack: &'static str,
    evidence: &'static str,
}

const PROBES: &[Probe] = &[
    Probe {
        file: "next.config.js",
        needle: None,
        stack: "nextjs",
        evidence: "next.config.js",
    },
    Probe {
        file: "next.config.mjs",
        needle: None,
        stack: "nextjs",
        evidence: "next.config.mjs",
    },
    Probe {
        file: "next.config.ts",
        needle: None,
        stack: "nextjs",
        evidence: "next.config.ts",
    },
    Probe {
        file: "components.json",
        needle: None,
        stack: "shadcn",
        evidence: "components.json (shadcn/ui)",
    },
    Probe {
        file: "package.json",
        needle: Some("\"next\""),
        stack: "nextjs",
        evidence: "next dependency in package.json",
    },
    Probe {
        file: "package.json",
        needle: Some("\"react\""),
        stack: "react",
        evidence: "react dependency in package.json",
    },
    Probe {
        file: "package.json",
        needle: Some("\"@playwright/test\""),
        stack: "playwright",
        evidence: "@playwright/test in package.json",
    },
    Probe {
        file: "package.json",
        needle: Some("\"express\""),
        stack: "node-api",
        evidence: "express in package.json",
    },
    Probe {
        file: "package.json",
        needle: Some("\"hono\""),
        stack: "node-api",
        evidence: "hono in package.json",
    },
    Probe {
        file: "go.mod",
        needle: None,
        stack: "go",
        evidence: "go.mod",
    },
    Probe {
        file: "pyproject.toml",
        needle: Some("fastapi"),
        stack: "fastapi",
        evidence: "fastapi in pyproject.toml",
    },
    Probe {
        file: "requirements.txt",
        needle: Some("fastapi"),
        stack: "fastapi",
        evidence: "fastapi in requirements.txt",
    },
    Probe {
        file: "manage.py",
        needle: None,
        stack: "django",
        evidence: "manage.py (Django)",
    },
    Probe {
        file: "pyproject.toml",
        needle: Some("django"),
        stack: "django",
        evidence: "django in pyproject.toml",
    },
    Probe {
        file: "Gemfile",
        needle: Some("rails"),
        stack: "rails",
        evidence: "rails in Gemfile",
    },
    Probe {
        file: "pom.xml",
        needle: None,
        stack: "java",
        evidence: "pom.xml",
    },
    Probe {
        file: "build.gradle",
        needle: None,
        stack: "java",
        evidence: "build.gradle",
    },
    Probe {
        file: "Cargo.toml",
        needle: None,
        stack: "rust",
        evidence: "Cargo.toml",
    },
    Probe {
        file: "tsconfig.json",
        needle: None,
        stack: "typescript",
        evidence: "tsconfig.json",
    },
    Probe {
        file: "svelte.config.js",
        needle: None,
        stack: "svelte",
        evidence: "svelte.config.js",
    },
    Probe {
        file: "package.json",
        needle: Some("\"vue\""),
        stack: "vue",
        evidence: "vue dependency in package.json",
    },
    Probe {
        file: "package.json",
        needle: Some("\"vitest\""),
        stack: "vitest",
        evidence: "vitest in package.json",
    },
];

/// Directories that never contain a project we should probe.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    "vendor",
    "out",
    ".git",
];

/// Inspect marker files to infer the stack(s). Scans the repo root AND its
/// immediate subdirectories, so monorepos (e.g. a Cargo workspace with a Next.js
/// app under `web/`) are detected rather than only single-language roots.
pub fn detect_stack(repo_root: &Path) -> DetectedStack {
    let mut dirs = vec![repo_root.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(repo_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()) {
                continue;
            }
            dirs.push(entry.path());
        }
    }

    let mut result = DetectedStack::default();
    for dir in &dirs {
        let prefix = if dir == repo_root {
            String::new()
        } else {
            format!(
                "{}/",
                dir.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            )
        };
        for probe in PROBES {
            let Ok(data) = std::fs::read_to_string(dir.join(probe.file)) else {
                continue;
            };
            if let Some(needle) = probe.needle {
                if !data.contains(needle) {
                    continue;
                }
            }
            if !result.stacks.iter().any(|s| s == probe.stack) {
                result.stacks.push(probe.stack.to_string());
            }
            let ev = format!("{prefix}{}", probe.evidence);
            if !result.evidence.contains(&ev) {
                result.evidence.push(ev);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rust_and_nested_nextjs_monorepo() {
        let dir = std::env::temp_dir().join(format!("sr-stack-{}", std::process::id()));
        let web = dir.join("web");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(web.join("next.config.mjs"), "export default {}\n").unwrap();
        std::fs::write(
            web.join("package.json"),
            "{\"dependencies\":{\"react\":\"19\",\"next\":\"15\"}}",
        )
        .unwrap();
        std::fs::write(web.join("tsconfig.json"), "{}").unwrap();

        let s = detect_stack(&dir);
        for expected in ["rust", "nextjs", "react", "typescript"] {
            assert!(
                s.stacks.iter().any(|x| x == expected),
                "missing {expected} in {:?}",
                s.stacks
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
