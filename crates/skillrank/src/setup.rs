//! `skillrank setup` — register the MCP server, Skill, and slash command with
//! Claude Code and Codex so the agent uses skillrank automatically. Writes
//! directly to the config files and global user skill/command paths
//! (idempotent, backed up) so it works even if the agent CLIs are not on PATH.
//!
//! The Skill and command files follow the ownership rules in [`crate::managed`]:
//! never clobber a hand edit, always back up before replacing, and never
//! re-create something the user deleted. `--force` overrides both refusals.

use crate::flags::Flags;
use crate::managed::{self, backup, Kind, Outcome, Policy, State, Triggers};
use serde_json::{json, Map, Value};
use skillrank_core::config;
use std::path::{Path, PathBuf};

/// Labels shared by `setup` and `update` so the two never describe the same
/// file differently.
const CLAUDE_SKILL_LABEL: &str = "skillrank Skill for Claude Code";
const CLAUDE_COMMAND_LABEL: &str = "/skillrank command for Claude Code";
const CODEX_SKILL_LABEL: &str = "skillrank Skill for Codex";
const CODEX_COMMAND_LABEL: &str = "/skillrank command for Codex";

#[derive(Clone, Copy)]
struct SetupParts {
    mcp: bool,
    skill: bool,
    command: bool,
}

/// A default path, or the reason there is nowhere to put it. Every default
/// hangs off the user's home directory, and a machine without one has no safe
/// answer — see [`resolve_home`].
type Resolved = Result<PathBuf, String>;

struct AgentPaths {
    config: Resolved,
    skill: Resolved,
    command: Resolved,
}

pub fn run(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let self_path = self_path();

    // The trigger preference is sticky: an explicit `--triggers` is recorded so
    // every later `setup` and `update` honours it, which is the only way an off
    // switch is worth having.
    let mut state = managed::load_default_state();
    let requested = f.value("triggers").trim().to_string();
    let requested = if requested.is_empty() {
        // A bare `--triggers` is a typo, and silently ignoring it would leave
        // someone believing they had turned the agent-initiated trigger off.
        if f.bool("triggers") {
            eprintln!("--triggers needs a value (expected: default | user-only)");
            return 2;
        }
        None
    } else {
        match Triggers::parse(&requested) {
            Some(t) => Some(t),
            None => {
                eprintln!("unknown --triggers value {requested:?} (expected: default | user-only)");
                return 2;
            }
        }
    };
    let triggers = state.resolve_triggers(requested);
    // Recorded as a decision, not left implicit: whatever this run installs is
    // what the machine is now configured for, and a later `update` reading a
    // recorded value never has to fall back to a default.
    state.triggers = Some(triggers);
    let force = f.bool("force");

    let claude_config = if !f.value("claude-config").is_empty() {
        Ok(PathBuf::from(f.value("claude-config")))
    } else {
        default_claude_config_path()
    };
    let codex_config = if !f.value("codex-config").is_empty() {
        Ok(PathBuf::from(f.value("codex-config")))
    } else {
        default_codex_config_path()
    };
    let claude_base = default_claude_base_path();
    let codex_base = default_codex_base_path();
    let claude_paths = AgentPaths {
        config: claude_config,
        skill: under(&claude_base, claude_skill_path),
        command: under(&claude_base, claude_command_path),
    };
    let codex_paths = AgentPaths {
        config: codex_config,
        skill: under(&codex_base, codex_skill_path),
        command: under(&codex_base, codex_command_path),
    };
    let api_url = f.value("api-url").trim().to_string();
    let parts = SetupParts {
        mcp: !f.bool("no-mcp"),
        skill: !f.bool("no-skill"),
        command: !f.bool("no-command"),
    };

    if f.bool("print") {
        if parts.skill {
            println!("Skill trigger variant: {}\n", triggers.describe());
        }
        if !f.bool("no-claude") {
            print_claude_plan(parts, &claude_paths, &self_path, &api_url);
        }
        if !f.bool("no-codex") {
            print_codex_plan(parts, &codex_paths, &self_path, &api_url);
        }
        return 0;
    }

    let mut rc = 0;
    // The reason a path could not be derived is the same for every target on
    // the same machine, so it is spelled out once and referred to after that.
    let mut explained = false;
    let mut ctx = InstallCtx {
        triggers,
        force,
        state: &mut state,
    };
    if !f.bool("no-claude") {
        if parts.mcp {
            rc |= with_path(
                &claude_paths.config,
                "Claude Code MCP",
                &mut explained,
                |p| match ensure_claude_mcp(p, &self_path, &api_url) {
                    Ok(_) => {
                        println!(
                            "✓ Registered skillrank MCP with Claude Code ({})",
                            p.display()
                        );
                        0
                    }
                    Err(e) => {
                        eprintln!("Claude Code MCP: {e}");
                        1
                    }
                },
            );
        }
        if parts.skill {
            rc |= with_path(
                &claude_paths.skill,
                CLAUDE_SKILL_LABEL,
                &mut explained,
                |p| install_managed(p, Kind::Skill, CLAUDE_SKILL_LABEL, &mut ctx),
            );
        }
        if parts.command {
            rc |= with_path(
                &claude_paths.command,
                CLAUDE_COMMAND_LABEL,
                &mut explained,
                |p| install_managed(p, Kind::Command, CLAUDE_COMMAND_LABEL, &mut ctx),
            );
        }
    }
    if !f.bool("no-codex") {
        if parts.mcp {
            rc |= with_path(&codex_paths.config, "Codex MCP", &mut explained, |p| {
                match ensure_codex_mcp(p, &self_path, &api_url) {
                    Ok(_) => {
                        println!("✓ Registered skillrank MCP with Codex ({})", p.display());
                        0
                    }
                    Err(e) => {
                        eprintln!("Codex MCP: {e}");
                        1
                    }
                }
            });
        }
        if parts.skill {
            rc |= with_path(&codex_paths.skill, CODEX_SKILL_LABEL, &mut explained, |p| {
                install_managed(p, Kind::Skill, CODEX_SKILL_LABEL, &mut ctx)
            });
        }
        if parts.command {
            rc |= with_path(
                &codex_paths.command,
                CODEX_COMMAND_LABEL,
                &mut explained,
                |p| install_managed(p, Kind::Command, CODEX_COMMAND_LABEL, &mut ctx),
            );
        }
    }
    // Record what was installed and which variant, even on a partial failure:
    // the bookkeeping is what makes a later deletion recognisable as deliberate.
    //
    // A failure here is reported, never swallowed. An unrecorded
    // `--triggers=user-only` is an off switch that turns itself back on at the
    // next update, and the moment it fails is the only moment the user can do
    // anything about it. The one thing not worth saying is that there is no
    // home directory, on a run that has already said so in more useful words.
    if let Err(e) = managed::save_default_state(&state) {
        if !explained {
            eprintln!("Could not record skillrank's setup state in ~/.skillrank/setup.json: {e}");
            if requested.is_some() {
                eprintln!(
                    "--triggers={} will NOT survive the next `skillrank setup` or `skillrank \
                     update` until that is fixed (check the permissions on ~/.skillrank, or set \
                     SKILLRANK_HOME to a writable directory).",
                    triggers.as_str()
                );
            } else if rc == 0 {
                eprintln!(
                    "The files above were written correctly; skillrank just will not remember \
                     that it installed them, so a later deletion looks like a fresh machine."
                );
            }
        }
        // An explicit `--triggers` that could not be written down did not take
        // effect beyond this run, whether or not there was anything new to say.
        if requested.is_some() {
            rc = 1;
        }
    }
    if rc == 0 {
        print_success(parts);
        if parts.skill {
            print_trigger_note(triggers);
        }
        println!("(Claude Code prompts once to approve the tools; approve them.)");
        println!("To skip the prompt, add to ~/.claude/settings.json: {{\"permissions\":{{\"allow\":[\"mcp__skillrank\"]}}}}");
        maybe_capture_email(&f, &api_url);
    }
    rc
}

/// Derive a path from a base directory, carrying forward the reason when there
/// was no base to derive it from.
fn under(base: &Resolved, derive: fn(&Path) -> PathBuf) -> Resolved {
    base.as_ref()
        .map(|base| derive(base))
        .map_err(String::clone)
}

/// Do the work at a path that could be derived, or say why there was nowhere to
/// write. Refusing is a failure (`rc = 1`): the user asked for a file and did
/// not get one.
fn with_path(
    resolved: &Resolved,
    label: &str,
    explained: &mut bool,
    write: impl FnOnce(&Path) -> i32,
) -> i32 {
    match resolved {
        Ok(path) => write(path),
        Err(reason) => {
            eprintln!("{label}: skipped, nowhere to write it.");
            if !*explained {
                eprintln!("{reason}");
                *explained = true;
            }
            1
        }
    }
}

/// Everything the Skill/command writes need, threaded through so the per-agent
/// call sites stay one line each.
struct InstallCtx<'a> {
    triggers: Triggers,
    force: bool,
    state: &'a mut State,
}

/// Install one managed file and say what happened. Only an I/O failure is an
/// error: declining to overwrite a hand edit, or to resurrect a file the user
/// deleted, is the feature working — reporting either as a failure would train
/// people to reach for `--force` by reflex.
fn install_managed(path: &Path, kind: Kind, label: &str, ctx: &mut InstallCtx) -> i32 {
    match managed::write_managed(
        path,
        kind,
        ctx.triggers,
        Policy::install(ctx.force),
        ctx.state,
    ) {
        Ok(written) => {
            println!("{}", install_line(written.outcome, label, path));
            0
        }
        Err(e) => {
            eprintln!("{label}: {e}");
            1
        }
    }
}

fn install_line(outcome: Outcome, label: &str, path: &Path) -> String {
    let path = path.display();
    match outcome {
        Outcome::Created => format!("✓ Installed {label} ({path})"),
        Outcome::Updated => format!("✓ Updated {label} ({path})"),
        Outcome::Unchanged => format!("✓ {label} already up to date ({path})"),
        Outcome::UserEdited => format!(
            "• Kept your edited {label} ({path}). Re-run with --force to replace it (the old copy is saved as {path}.skillrank-bak)."
        ),
        Outcome::UserRemoved => format!(
            "• You removed {label}, so it stays removed ({path}). Re-run with --force to reinstall it."
        ),
        // Unreachable with an install policy, which always creates; spelled out
        // rather than panicked on so a future policy change degrades quietly.
        Outcome::Absent => format!("• Skipped {label} ({path})"),
    }
}

/// Name the trigger variant and its off switch at the moment it takes effect —
/// on install, and on the `update` that first turns it on for an existing
/// install. Enabling a trigger that lets the agent query a third-party registry
/// on its own initiative is not something a user should have to read a diff to
/// discover.
pub(crate) fn print_trigger_note(triggers: Triggers) {
    println!("{}", trigger_note(triggers));
}

fn trigger_note(triggers: Triggers) -> &'static str {
    match triggers {
        Triggers::Situational => {
            "The agent may now also check skillrank on its own — before working with a tool it has no approach for, or after failing twice at the same thing. It suggests, it never installs without your yes. Turn that off for good with `skillrank setup --triggers=user-only`."
        }
        Triggers::UserOnly => {
            "Trigger variant: user-only — the agent only reaches for skillrank when you ask. `skillrank setup --triggers=default` turns the agent-initiated trigger back on."
        }
    }
}

/// Every Skill/command file this machine has, in install order. `update` reuses
/// it so a post-update refresh can never drift from what `setup` wrote.
///
/// Fallible for the same reason `setup` is: these paths only mean anything
/// relative to a real home directory, and inventing one puts skillrank's files
/// inside the caller's working directory.
pub fn managed_targets() -> Result<Vec<managed::Target>, String> {
    let claude = default_claude_base_path()?;
    let codex = default_codex_base_path()?;
    Ok(vec![
        managed::Target {
            label: CLAUDE_SKILL_LABEL.to_string(),
            path: claude_skill_path(&claude),
            kind: Kind::Skill,
        },
        managed::Target {
            label: CLAUDE_COMMAND_LABEL.to_string(),
            path: claude_command_path(&claude),
            kind: Kind::Command,
        },
        managed::Target {
            label: CODEX_SKILL_LABEL.to_string(),
            path: codex_skill_path(&codex),
            kind: Kind::Skill,
        },
        managed::Target {
            label: CODEX_COMMAND_LABEL.to_string(),
            path: codex_command_path(&codex),
            kind: Kind::Command,
        },
    ])
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

fn default_claude_config_path() -> Resolved {
    Ok(default_home_path()?.join(".claude.json"))
}

fn default_codex_config_path() -> Resolved {
    Ok(default_codex_base_path()?.join("config.toml"))
}

fn default_claude_base_path() -> Resolved {
    Ok(default_home_path()?.join(".claude"))
}

fn default_codex_base_path() -> Resolved {
    if let Ok(h) = std::env::var("CODEX_HOME") {
        if !h.trim().is_empty() {
            return Ok(PathBuf::from(h));
        }
    }
    Ok(default_home_path()?.join(".codex"))
}

/// What to do about it, said once, wherever the home directory came up short.
const HOME_GUIDANCE: &str = "  Set HOME, or name the files explicitly:
    skillrank setup --claude-config /path/to/.claude.json \\
                    --codex-config /path/to/config.toml --no-skill --no-command
  CODEX_HOME supplies the Codex paths on its own.";

/// The home directory every default path is derived from.
///
/// Deliberately fallible. This used to fall back to `.`, which turned
/// `skillrank update` — a command whose only job is to check for a newer binary
/// — into something that rewrote git-tracked files in whatever repository the
/// user happened to be standing in. `env -i`, systemd units, cron jobs and many
/// containers all run with no HOME, so that is an ordinary condition, not an
/// exotic one; the current directory is never the right answer to it.
///
/// A relative HOME is refused for the same reason: it names a different
/// directory depending on where the process was started, which is exactly the
/// property that made `.` dangerous.
fn resolve_home(candidate: Option<PathBuf>) -> Resolved {
    let Some(home) = candidate else {
        return Err(format!(
            "could not determine your home directory: HOME and USERPROFILE are both unset. \
             Every file skillrank owns lives under it, and falling back to the current \
             directory would write them into whatever repository you are standing in.\n\
             {HOME_GUIDANCE}"
        ));
    };
    if !home.is_absolute() {
        return Err(format!(
            "HOME is {}, which is not an absolute path, so it names a different directory \
             depending on where skillrank is run from.\n{HOME_GUIDANCE}",
            home.display()
        ));
    }
    Ok(home)
}

fn default_home_path() -> Resolved {
    resolve_home(config::home_dir())
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

/// A path for `--print`, or the reason there is none. `--print` reports a plan
/// and must not pretend the plan is achievable when it is not.
fn show(resolved: &Resolved) -> String {
    match resolved {
        Ok(path) => path.display().to_string(),
        Err(reason) => format!(
            "<unavailable: {}>",
            reason.lines().next().unwrap_or_default()
        ),
    }
}

fn print_claude_plan(parts: SetupParts, paths: &AgentPaths, self_path: &str, api_url: &str) {
    if parts.mcp {
        println!(
            "Claude Code ({}) — add under \"mcpServers\":",
            show(&paths.config)
        );
        println!(
            "  \"skillrank\": {}\n",
            claude_entry_json(self_path, api_url)
        );
    }
    if parts.skill {
        println!("Claude Code Skill — write {}", show(&paths.skill));
    }
    if parts.command {
        println!(
            "Claude Code /skillrank command — write {}",
            show(&paths.command)
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
            show(&paths.config),
            codex_block(self_path, api_url)
        );
    }
    if parts.skill {
        println!("Codex Skill — write {}", show(&paths.skill));
    }
    if parts.command {
        println!("Codex /skillrank command — write {}", show(&paths.command));
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

    /// Install the Skill + command the way `run` does, twice, and report the
    /// second run's outcomes so idempotency is observable.
    fn install_twice(skill: &Path, command: &Path, triggers: Triggers) -> (Outcome, Outcome) {
        let mut state = State::default();
        let mut install = |path: &Path, kind| {
            managed::write_managed(path, kind, triggers, Policy::install(false), &mut state)
                .unwrap()
        };
        install(skill, Kind::Skill);
        install(command, Kind::Command);
        (
            install(skill, Kind::Skill).outcome,
            install(command, Kind::Command).outcome,
        )
    }

    #[test]
    fn installs_claude_skill_and_command_under_user_home() {
        let home = tmp("claude-assets");
        let base = home.join(".claude");
        let skill_path = claude_skill_path(&base);
        let command_path = claude_command_path(&base);

        let (skill, command) = install_twice(&skill_path, &command_path, Triggers::Situational);
        assert_eq!(skill, Outcome::Unchanged);
        assert_eq!(command, Outcome::Unchanged);

        assert_eq!(skill_path, home.join(".claude/skills/skillrank/SKILL.md"));
        assert_eq!(command_path, home.join(".claude/commands/skillrank.md"));
        assert_eq!(
            std::fs::read_to_string(&skill_path).unwrap(),
            managed::SKILL_MD
        );
        assert_eq!(
            std::fs::read_to_string(&command_path).unwrap(),
            managed::COMMAND_MD
        );
        // A second install is a no-op, so nothing extra (least of all a
        // gratuitous .skillrank-bak) appears next to the files.
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

        let (skill, command) = install_twice(&skill_path, &command_path, Triggers::Situational);
        assert_eq!(skill, Outcome::Unchanged);
        assert_eq!(command, Outcome::Unchanged);

        assert_eq!(skill_path, home.join(".codex/skills/skillrank/SKILL.md"));
        assert_eq!(command_path, home.join(".codex/prompts/skillrank.md"));
        assert_eq!(
            std::fs::read_to_string(&skill_path).unwrap(),
            managed::SKILL_MD
        );
        assert_eq!(
            std::fs::read_to_string(&command_path).unwrap(),
            managed::COMMAND_MD
        );
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

    #[test]
    fn the_off_switch_writes_the_user_initiated_description() {
        let home = tmp("user-only");
        let skill_path = claude_skill_path(&home.join(".claude"));

        let (skill, _) = install_twice(
            &skill_path,
            &claude_command_path(&home.join(".claude")),
            Triggers::UserOnly,
        );

        assert_eq!(
            skill,
            Outcome::Unchanged,
            "user-only must be idempotent too"
        );
        assert_eq!(
            std::fs::read_to_string(&skill_path).unwrap(),
            managed::SKILL_MD_USER_ONLY
        );
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn managed_targets_cover_both_agents_skill_and_command() {
        let targets = managed_targets().expect("a test machine has a home directory");
        assert_eq!(targets.len(), 4);
        assert_eq!(targets.iter().filter(|t| t.kind == Kind::Skill).count(), 2);
        assert_eq!(
            targets.iter().filter(|t| t.kind == Kind::Command).count(),
            2
        );
        for target in &targets {
            assert!(target.path.ends_with("SKILL.md") || target.path.ends_with("skillrank.md"));
        }
    }

    /// `setup` with no flags must not touch anything inside a repository — a
    /// teammate who never installed skillrank should never find its edits in a
    /// diff, and `update`, whose only job is to check for a newer binary, least
    /// of all.
    ///
    /// The assertion is that every default path is *absolute* and rooted in a
    /// real home directory. Comparing against a prefix derived the same way the
    /// path was would pass just as happily on `./.claude/...`, which is exactly
    /// the bug: a `.` fallback made every path relative to whatever directory
    /// the process was started in.
    #[test]
    fn setup_never_writes_inside_a_repository() {
        let home = default_home_path().expect("a test machine has a home directory");
        assert!(home.is_absolute(), "{} is not absolute", home.display());
        let cwd = std::env::current_dir().expect("cwd");

        let claude = default_claude_base_path().unwrap();
        let codex = default_codex_base_path().unwrap();
        let paths: Vec<PathBuf> = managed_targets()
            .unwrap()
            .into_iter()
            .map(|target| target.path)
            .chain([
                default_claude_config_path().unwrap(),
                default_codex_config_path().unwrap(),
            ])
            .collect();

        for path in &paths {
            assert!(path.is_absolute(), "{} is a relative path", path.display());
            assert!(
                path.starts_with(&claude) || path.starts_with(&codex) || path.starts_with(&home),
                "{} is not rooted in a resolved home directory",
                path.display()
            );
            // Belt and braces, in the terms a teammate would notice it in.
            assert!(
                !path.starts_with(&cwd) || cwd.starts_with(&home),
                "{} lands inside the working directory",
                path.display()
            );
        }
    }

    /// The case the prefix-comparison guard could not see: with no home
    /// directory at all, deriving a path must fail rather than produce one
    /// relative to the caller's working directory.
    #[test]
    fn no_home_directory_is_an_error_not_a_relative_path() {
        let cleared = resolve_home(None).expect_err("HOME unset must not resolve");
        assert!(cleared.contains("HOME"), "{cleared}");
        // The message has to say what to do instead, not just what went wrong.
        assert!(cleared.contains("--claude-config"), "{cleared}");
        assert!(cleared.contains("--codex-config"), "{cleared}");

        // A relative HOME is the same hole wearing a different hat: it names a
        // different directory depending on where skillrank was started.
        for relative in [".", "", "relative/home"] {
            assert!(
                resolve_home(Some(PathBuf::from(relative))).is_err(),
                "HOME={relative:?} resolved instead of failing"
            );
        }

        let absolute = resolve_home(Some(PathBuf::from("/somewhere/else"))).unwrap();
        assert_eq!(absolute, PathBuf::from("/somewhere/else"));
    }

    /// Whichever variant is in force, the note has to name it and name the one
    /// command that switches to the other. A disclosure nobody can act on is
    /// not a disclosure.
    #[test]
    fn each_trigger_note_names_its_own_off_switch() {
        let situational = trigger_note(Triggers::Situational);
        assert!(situational.contains("on its own"), "{situational}");
        assert!(
            situational.contains("never installs without your yes"),
            "{situational}"
        );
        assert!(
            situational.contains("skillrank setup --triggers=user-only"),
            "{situational}"
        );

        let user_only = trigger_note(Triggers::UserOnly);
        assert!(
            user_only.contains("only reaches for skillrank when you ask"),
            "{user_only}"
        );
        assert!(
            user_only.contains("skillrank setup --triggers=default"),
            "{user_only}"
        );
    }

    #[test]
    fn install_lines_explain_the_refusals() {
        let path = Path::new("/tmp/SKILL.md");
        assert!(install_line(Outcome::Created, "x", path).starts_with("✓ Installed"));
        assert!(install_line(Outcome::Updated, "x", path).starts_with("✓ Updated"));
        assert!(install_line(Outcome::Unchanged, "x", path).contains("already up to date"));
        // A refusal is only useful if it says how to override it.
        assert!(install_line(Outcome::UserEdited, "x", path).contains("--force"));
        assert!(install_line(Outcome::UserEdited, "x", path).contains(".skillrank-bak"));
        assert!(install_line(Outcome::UserRemoved, "x", path).contains("--force"));
    }
}
