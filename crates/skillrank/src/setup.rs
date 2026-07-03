//! `skillrank setup` — register the MCP server with Claude Code and Codex so the
//! agent uses skillrank automatically. Writes directly to the config files
//! (idempotent, backed up) so it works even if the agent CLIs are not on PATH.

use crate::flags::Flags;
use serde_json::{json, Map, Value};
use skillrank_core::config;
use std::path::{Path, PathBuf};

pub fn run(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let self_path = self_path();

    let claude_path = if !f.value("claude-config").is_empty() {
        PathBuf::from(f.value("claude-config"))
    } else {
        default_claude_config_path()
    };
    let codex_path = if !f.value("codex-config").is_empty() {
        PathBuf::from(f.value("codex-config"))
    } else {
        default_codex_config_path()
    };
    let api_url = f.value("api-url").trim().to_string();

    if f.bool("print") {
        println!(
            "Claude Code ({}) — add under \"mcpServers\":",
            claude_path.display()
        );
        println!(
            "  \"skillrank\": {}\n",
            claude_entry_json(&self_path, &api_url)
        );
        println!(
            "Codex ({}) — append:\n{}",
            codex_path.display(),
            codex_block(&self_path, &api_url)
        );
        return 0;
    }

    let mut rc = 0;
    if !f.bool("no-claude") {
        match ensure_claude_mcp(&claude_path, &self_path, &api_url) {
            Ok(_) => println!(
                "✓ Registered skillrank MCP with Claude Code ({})",
                claude_path.display()
            ),
            Err(e) => {
                eprintln!("Claude Code: {e}");
                rc = 1;
            }
        }
    }
    if !f.bool("no-codex") {
        match ensure_codex_mcp(&codex_path, &self_path, &api_url) {
            Ok(_) => println!(
                "✓ Registered skillrank MCP with Codex ({})",
                codex_path.display()
            ),
            Err(e) => {
                eprintln!("Codex: {e}");
                rc = 1;
            }
        }
    }
    if rc == 0 {
        println!(
            "\nDone. Restart your agent, then just ask it to find, install, or evaluate skills —"
        );
        println!("no commands to remember. (Claude Code prompts once to approve the tools; approve them.)");
        println!("To skip the prompt, add to ~/.claude/settings.json: {{\"permissions\":{{\"allow\":[\"mcp__skillrank\"]}}}}");
    }
    rc
}

fn self_path() -> String {
    match std::env::current_exe() {
        Ok(p) => std::fs::canonicalize(&p)
            .unwrap_or(p)
            .to_string_lossy()
            .to_string(),
        Err(_) => "skillrank".to_string(),
    }
}

fn default_claude_config_path() -> PathBuf {
    config::home_dir()
        .map(|h| h.join(".claude.json"))
        .unwrap_or_else(|| PathBuf::from(".claude.json"))
}

fn default_codex_config_path() -> PathBuf {
    if let Ok(h) = std::env::var("CODEX_HOME") {
        if !h.trim().is_empty() {
            return PathBuf::from(h).join("config.toml");
        }
    }
    config::home_dir()
        .map(|h| h.join(".codex").join("config.toml"))
        .unwrap_or_else(|| PathBuf::from(".codex/config.toml"))
}

pub fn claude_entry(self_path: &str, api_url: &str) -> Value {
    let mut entry = json!({
        "type": "stdio",
        "command": self_path,
        "args": ["mcp"],
    });
    if !api_url.is_empty() {
        entry["env"] = json!({ "SKILLRANK_API_URL": api_url });
    }
    entry
}

fn claude_entry_json(self_path: &str, api_url: &str) -> String {
    claude_entry(self_path, api_url).to_string()
}

pub fn codex_block(self_path: &str, api_url: &str) -> String {
    let mut block = format!("[mcp_servers.skillrank]\ncommand = {self_path:?}\nargs = [\"mcp\"]\n");
    if !api_url.is_empty() {
        block.push_str(&format!(
            "[mcp_servers.skillrank.env]\nSKILLRANK_API_URL = {api_url:?}\n"
        ));
    }
    block
}

/// Merge an mcpServers.skillrank entry into ~/.claude.json, preserving all other
/// data. Backs up the file first.
pub fn ensure_claude_mcp(path: &Path, self_path: &str, api_url: &str) -> std::io::Result<()> {
    let mut doc: Map<String, Value> = Map::new();
    if let Ok(data) = std::fs::read_to_string(path) {
        if !data.trim().is_empty() {
            doc = serde_json::from_str(&data).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("parse {}: {e}", path.display()),
                )
            })?;
            backup(path, data.as_bytes())?;
        }
    }
    let servers = doc
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(map) = servers {
        map.insert("skillrank".to_string(), claude_entry(self_path, api_url));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = serde_json::to_string_pretty(&doc).map_err(std::io::Error::other)?;
    out.push('\n');
    std::fs::write(path, out)
}

/// Write the [mcp_servers.skillrank] block, replacing any prior skillrank block
/// (so re-running updates it) and preserving everything else.
pub fn ensure_codex_mcp(path: &Path, self_path: &str, api_url: &str) -> std::io::Result<()> {
    let mut existing = String::new();
    if let Ok(data) = std::fs::read_to_string(path) {
        existing = strip_codex_skillrank_block(&data);
        backup(path, data.as_bytes())?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = existing.trim_end_matches('\n').to_string();
    if !out.trim().is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&codex_block(self_path, api_url));
    std::fs::write(path, out)
}

/// Remove any [mcp_servers.skillrank] and [mcp_servers.skillrank.env] tables,
/// leaving all other config intact.
fn strip_codex_skillrank_block(s: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in s.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            skipping =
                trimmed == "[mcp_servers.skillrank]" || trimmed == "[mcp_servers.skillrank.env]";
        }
        if !skipping {
            out.push(line);
        }
    }
    out.join("\n")
}

fn backup(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let bak = format!("{}.skillrank-bak", path.display());
    std::fs::write(bak, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("skillrank-setup-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn codex_preserves_and_is_idempotent() {
        let dir = tmp("codex");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[mcp_servers.playwright]\ncommand = \"npx\"\nargs = [\"@playwright/mcp@latest\"]\n",
        )
        .unwrap();
        ensure_codex_mcp(&path, "/usr/local/bin/skillrank", "").unwrap();
        let s = std::fs::read_to_string(&path).unwrap();
        assert!(
            s.contains("[mcp_servers.playwright]"),
            "existing server lost"
        );
        assert!(s.contains("[mcp_servers.skillrank]"));
        ensure_codex_mcp(&path, "/usr/local/bin/skillrank", "").unwrap();
        let s = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            s.matches("[mcp_servers.skillrank]").count(),
            1,
            "duplicate section"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn claude_merges_and_injects_api_url() {
        let dir = tmp("claude");
        let path = dir.join("claude.json");
        std::fs::write(
            &path,
            r#"{"numStartups":42,"mcpServers":{"context7":{"command":"npx"}}}"#,
        )
        .unwrap();
        ensure_claude_mcp(&path, "/usr/local/bin/skillrank", "http://localhost:8899").unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc["numStartups"], 42);
        assert!(doc["mcpServers"]["context7"].is_object(), "context7 lost");
        assert_eq!(
            doc["mcpServers"]["skillrank"]["env"]["SKILLRANK_API_URL"],
            "http://localhost:8899"
        );
        assert!(
            path.with_extension("json.skillrank-bak").exists()
                || std::path::Path::new(&format!("{}.skillrank-bak", path.display())).exists()
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
