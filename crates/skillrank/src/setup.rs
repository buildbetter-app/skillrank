//! `skillrank setup` — register the MCP server, Skill, and slash command with
//! Claude Code and Codex so the agent uses skillrank automatically. Writes
//! directly to the config files and global user skill/command paths
//! (idempotent, backed up) so it works even if the agent CLIs are not on PATH.

use crate::flags::Flags;
use serde_json::{json, Map, Value};
use skillrank_core::config;
use std::path::{Path, PathBuf};

const SKILL_MD: &str = include_str!("skillrank_skill.md");
const COMMAND_MD: &str = include_str!("skillrank_command.md");

#[derive(Clone, Copy)]
struct SetupParts {
    mcp: bool,
    skill: bool,
    command: bool,
}

struct AgentPaths {
    config: PathBuf,
    skill: PathBuf,
    command: PathBuf,
}

pub fn run(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let self_path = self_path();

    let claude_config = if !f.value("claude-config").is_empty() {
        PathBuf::from(f.value("claude-config"))
    } else {
        default_claude_config_path()
    };
    let codex_config = if !f.value("codex-config").is_empty() {
        PathBuf::from(f.value("codex-config"))
    } else {
        default_codex_config_path()
    };
    let claude_base = default_claude_base_path();
    let codex_base = default_codex_base_path();
    let claude_paths = AgentPaths {
        config: claude_config,
        skill: claude_skill_path(&claude_base),
        command: claude_command_path(&claude_base),
    };
    let codex_paths = AgentPaths {
        config: codex_config,
        skill: codex_skill_path(&codex_base),
        command: codex_command_path(&codex_base),
    };
    let api_url = f.value("api-url").trim().to_string();
    let parts = SetupParts {
        mcp: !f.bool("no-mcp"),
        skill: !f.bool("no-skill"),
        command: !f.bool("no-command"),
    };

    if f.bool("print") {
        if !f.bool("no-claude") {
            print_claude_plan(parts, &claude_paths, &self_path, &api_url);
        }
        if !f.bool("no-codex") {
            print_codex_plan(parts, &codex_paths, &self_path, &api_url);
        }
        return 0;
    }

    let mut rc = 0;
    if !f.bool("no-claude") {
        if parts.mcp {
            match ensure_claude_mcp(&claude_paths.config, &self_path, &api_url) {
                Ok(_) => println!(
                    "✓ Registered skillrank MCP with Claude Code ({})",
                    claude_paths.config.display()
                ),
                Err(e) => {
                    eprintln!("Claude Code MCP: {e}");
                    rc = 1;
                }
            }
        }
        if parts.skill {
            match ensure_skill(&claude_paths.skill) {
                Ok(_) => println!(
                    "✓ Installed skillrank Skill for Claude Code ({})",
                    claude_paths.skill.display()
                ),
                Err(e) => {
                    eprintln!("Claude Code Skill: {e}");
                    rc = 1;
                }
            }
        }
        if parts.command {
            match ensure_command(&claude_paths.command) {
                Ok(_) => println!(
                    "✓ Installed /skillrank command for Claude Code ({})",
                    claude_paths.command.display()
                ),
                Err(e) => {
                    eprintln!("Claude Code command: {e}");
                    rc = 1;
                }
            }
        }
    }
    if !f.bool("no-codex") {
        if parts.mcp {
            match ensure_codex_mcp(&codex_paths.config, &self_path, &api_url) {
                Ok(_) => println!(
                    "✓ Registered skillrank MCP with Codex ({})",
                    codex_paths.config.display()
                ),
                Err(e) => {
                    eprintln!("Codex MCP: {e}");
                    rc = 1;
                }
            }
        }
        if parts.skill {
            match ensure_skill(&codex_paths.skill) {
                Ok(_) => println!(
                    "✓ Installed skillrank Skill for Codex ({})",
                    codex_paths.skill.display()
                ),
                Err(e) => {
                    eprintln!("Codex Skill: {e}");
                    rc = 1;
                }
            }
        }
        if parts.command {
            match ensure_command(&codex_paths.command) {
                Ok(_) => println!(
                    "✓ Installed /skillrank command for Codex ({})",
                    codex_paths.command.display()
                ),
                Err(e) => {
                    eprintln!("Codex command: {e}");
                    rc = 1;
                }
            }
        }
    }
    if rc == 0 {
        print_success(parts);
        println!("(Claude Code prompts once to approve the tools; approve them.)");
        println!("To skip the prompt, add to ~/.claude/settings.json: {{\"permissions\":{{\"allow\":[\"mcp__skillrank\"]}}}}");
        maybe_capture_email(&f, &api_url);
    }
    rc
}

/// Optionally record an email for occasional skill updates. Uses `--email` when
/// given; otherwise prompts ONLY when stdin is an interactive terminal, so a
/// piped `curl | sh` install never blocks. Fully skippable via `--no-email` or
/// SKILLRANK_NO_EMAIL. Best-effort: a failure never fails setup.
fn maybe_capture_email(f: &Flags, api_url: &str) {
    use std::io::{IsTerminal, Write};
    if f.bool("no-email") || std::env::var_os("SKILLRANK_NO_EMAIL").is_some() {
        return;
    }
    let mut email = f.value("email").trim().to_string();
    if email.is_empty() {
        if !std::io::stdin().is_terminal() {
            return;
        }
        print!("\nEmail for occasional skill updates (optional, Enter to skip): ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return;
        }
        email = line.trim().to_string();
    }
    if email.is_empty() {
        return;
    }
    if !(email.contains('@') && email.contains('.')) {
        println!("Skipped: '{email}' doesn't look like an email.");
        return;
    }
    let client = skillrank_core::Client::new(if api_url.is_empty() {
        None
    } else {
        Some(api_url)
    });
    match client.subscribe_email(&email) {
        Ok(()) => println!("Thanks — occasional skill updates will go to {email}."),
        Err(e) => println!(
            "(Couldn't record your email right now: {e}. skillrank works fine regardless.)"
        ),
    }
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
    default_home_path().join(".claude.json")
}

fn default_codex_config_path() -> PathBuf {
    default_codex_base_path().join("config.toml")
}

fn default_claude_base_path() -> PathBuf {
    default_home_path().join(".claude")
}

fn default_codex_base_path() -> PathBuf {
    if let Ok(h) = std::env::var("CODEX_HOME") {
        if !h.trim().is_empty() {
            return PathBuf::from(h);
        }
    }
    default_home_path().join(".codex")
}

fn default_home_path() -> PathBuf {
    config::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn claude_skill_path(base: &Path) -> PathBuf {
    base.join("skills").join("skillrank").join("SKILL.md")
}

fn claude_command_path(base: &Path) -> PathBuf {
    base.join("commands").join("skillrank.md")
}

fn codex_skill_path(base: &Path) -> PathBuf {
    base.join("skills").join("skillrank").join("SKILL.md")
}

fn codex_command_path(base: &Path) -> PathBuf {
    base.join("prompts").join("skillrank.md")
}

fn print_claude_plan(parts: SetupParts, paths: &AgentPaths, self_path: &str, api_url: &str) {
    if parts.mcp {
        println!(
            "Claude Code ({}) — add under \"mcpServers\":",
            paths.config.display()
        );
        println!(
            "  \"skillrank\": {}\n",
            claude_entry_json(self_path, api_url)
        );
    }
    if parts.skill {
        println!("Claude Code Skill — write {}", paths.skill.display());
    }
    if parts.command {
        println!(
            "Claude Code /skillrank command — write {}",
            paths.command.display()
        );
    }
    if parts.skill || parts.command {
        println!();
    }
}

fn print_codex_plan(parts: SetupParts, paths: &AgentPaths, self_path: &str, api_url: &str) {
    if parts.mcp {
        println!(
            "Codex ({}) — append:\n{}",
            paths.config.display(),
            codex_block(self_path, api_url)
        );
    }
    if parts.skill {
        println!("Codex Skill — write {}", paths.skill.display());
    }
    if parts.command {
        println!(
            "Codex /skillrank command — write {}",
            paths.command.display()
        );
    }
}

fn print_success(parts: SetupParts) {
    if parts.mcp && parts.command && parts.skill {
        println!("\nDone. MCP registered + /skillrank command available + skill installed.");
        println!("Type `/skillrank recommend`, or just ask your agent to find/install skills. Restart the agent to load them.");
        return;
    }

    let mut installed = Vec::new();
    if parts.mcp {
        installed.push("MCP registered");
    }
    if parts.command {
        installed.push("/skillrank command available");
    }
    if parts.skill {
        installed.push("skill installed");
    }
    if installed.is_empty() {
        println!("\nDone. Nothing selected to install.");
    } else {
        println!("\nDone. {}.", installed.join(" + "));
        println!("Restart the agent to load the installed pieces.");
    }
}

pub fn ensure_skill(path: &Path) -> std::io::Result<()> {
    write_owned_file(path, SKILL_MD)
}

pub fn ensure_command(path: &Path) -> std::io::Result<()> {
    write_owned_file(path, COMMAND_MD)
}

fn write_owned_file(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
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

    #[test]
    fn installs_claude_skill_and_command_under_user_home() {
        let home = tmp("claude-assets");
        let base = home.join(".claude");
        let skill_path = claude_skill_path(&base);
        let command_path = claude_command_path(&base);

        ensure_skill(&skill_path).unwrap();
        ensure_command(&command_path).unwrap();
        ensure_skill(&skill_path).unwrap();
        ensure_command(&command_path).unwrap();

        assert_eq!(skill_path, home.join(".claude/skills/skillrank/SKILL.md"));
        assert_eq!(command_path, home.join(".claude/commands/skillrank.md"));
        assert_eq!(std::fs::read_to_string(&skill_path).unwrap(), SKILL_MD);
        assert_eq!(std::fs::read_to_string(&command_path).unwrap(), COMMAND_MD);
        assert_eq!(
            std::fs::read_dir(home.join(".claude/skills/skillrank"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            std::fs::read_dir(home.join(".claude/commands"))
                .unwrap()
                .count(),
            1
        );
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn installs_codex_skill_and_command_under_user_home() {
        let home = tmp("codex-assets");
        let base = home.join(".codex");
        let skill_path = codex_skill_path(&base);
        let command_path = codex_command_path(&base);

        ensure_skill(&skill_path).unwrap();
        ensure_command(&command_path).unwrap();
        ensure_skill(&skill_path).unwrap();
        ensure_command(&command_path).unwrap();

        assert_eq!(skill_path, home.join(".codex/skills/skillrank/SKILL.md"));
        assert_eq!(command_path, home.join(".codex/prompts/skillrank.md"));
        assert_eq!(std::fs::read_to_string(&skill_path).unwrap(), SKILL_MD);
        assert_eq!(std::fs::read_to_string(&command_path).unwrap(), COMMAND_MD);
        assert_eq!(
            std::fs::read_dir(home.join(".codex/skills/skillrank"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            std::fs::read_dir(home.join(".codex/prompts"))
                .unwrap()
                .count(),
            1
        );
        std::fs::remove_dir_all(&home).ok();
    }
}
