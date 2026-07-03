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
    Probe { file: "next.config.js", needle: None, stack: "nextjs", evidence: "next.config.js" },
    Probe { file: "next.config.mjs", needle: None, stack: "nextjs", evidence: "next.config.mjs" },
    Probe { file: "next.config.ts", needle: None, stack: "nextjs", evidence: "next.config.ts" },
    Probe { file: "components.json", needle: None, stack: "shadcn", evidence: "components.json (shadcn/ui)" },
    Probe { file: "package.json", needle: Some("\"next\""), stack: "nextjs", evidence: "next dependency in package.json" },
    Probe { file: "package.json", needle: Some("\"react\""), stack: "react", evidence: "react dependency in package.json" },
    Probe { file: "package.json", needle: Some("\"@playwright/test\""), stack: "playwright", evidence: "@playwright/test in package.json" },
    Probe { file: "package.json", needle: Some("\"express\""), stack: "node-api", evidence: "express in package.json" },
    Probe { file: "package.json", needle: Some("\"hono\""), stack: "node-api", evidence: "hono in package.json" },
    Probe { file: "go.mod", needle: None, stack: "go", evidence: "go.mod" },
    Probe { file: "pyproject.toml", needle: Some("fastapi"), stack: "fastapi", evidence: "fastapi in pyproject.toml" },
    Probe { file: "requirements.txt", needle: Some("fastapi"), stack: "fastapi", evidence: "fastapi in requirements.txt" },
    Probe { file: "manage.py", needle: None, stack: "django", evidence: "manage.py (Django)" },
    Probe { file: "pyproject.toml", needle: Some("django"), stack: "django", evidence: "django in pyproject.toml" },
    Probe { file: "Gemfile", needle: Some("rails"), stack: "rails", evidence: "rails in Gemfile" },
    Probe { file: "pom.xml", needle: None, stack: "java", evidence: "pom.xml" },
    Probe { file: "build.gradle", needle: None, stack: "java", evidence: "build.gradle" },
];

/// Inspect marker files at the repo root to infer the stack(s).
pub fn detect_stack(repo_root: &Path) -> DetectedStack {
    let mut result = DetectedStack::default();
    for probe in PROBES {
        let path = repo_root.join(probe.file);
        let Ok(data) = std::fs::read_to_string(&path) else {
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
        result.evidence.push(probe.evidence.to_string());
    }
    result
}
