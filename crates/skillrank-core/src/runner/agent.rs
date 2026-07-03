//! Invokes the user's own `claude`/`codex` binary as a one-shot on the user's own
//! subscription. The verifier is applied by the caller AFTER this returns, so the
//! agent workspace never contains verifier content.

use super::{AgentRunner, RunOutcome, RunSpec};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct CliAgentRunner {
    pub provider: String, // "claude" | "codex"
    pub binary: String,
    pub version: String,
}

impl AgentRunner for CliAgentRunner {
    fn agent_name(&self) -> String {
        if self.provider == "claude" {
            "claude_code".to_string()
        } else {
            self.provider.clone()
        }
    }

    fn agent_version_band(&self) -> String {
        version_band(&self.version)
    }

    fn run_task(&self, spec: &RunSpec) -> Result<RunOutcome, String> {
        if spec.skill_installed {
            install_skill_into_workspace(spec)?;
        }
        let prompt = build_prompt(spec);
        let timeout = Duration::from_secs(if spec.timeout_sec == 0 {
            240
        } else {
            spec.timeout_sec as u64
        });

        let mut child = Command::new(&self.binary)
            .args(self.argv(spec))
            .current_dir(&spec.working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", self.binary))?;

        // Drain stdout on a thread so a full pipe never deadlocks us.
        let mut stdout = child.stdout.take().expect("piped stdout");
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            buf
        });
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prompt.as_bytes());
            // stdin dropped here -> EOF for the child
        }

        let start = Instant::now();
        let mut timed_out = false;
        let mut success = false;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    success = status.success();
                    break;
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        timed_out = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => break,
            }
        }
        let elapsed = start.elapsed().as_millis() as i64;
        let stdout_bytes = reader.join().unwrap_or_default();
        let digest = format!("sha256:{:x}", Sha256::digest(&stdout_bytes));

        if timed_out || !success {
            return Ok(RunOutcome {
                duration_ms: elapsed,
                trajectory_digest: digest,
                agent_error: true,
                ..Default::default()
            });
        }
        match parse_agent_usage(&self.provider, &stdout_bytes) {
            Ok(mut outcome) => {
                outcome.duration_ms = elapsed;
                outcome.trajectory_digest = digest;
                Ok(outcome)
            }
            Err(_) => Ok(RunOutcome {
                duration_ms: elapsed,
                trajectory_digest: digest,
                agent_error: true,
                ..Default::default()
            }),
        }
    }
}

impl CliAgentRunner {
    fn argv(&self, spec: &RunSpec) -> Vec<String> {
        if self.provider == "claude" {
            let mut args = vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "json".to_string(),
                "--dangerously-skip-permissions".to_string(),
            ];
            if !spec.model.is_empty() {
                args.push("--model".into());
                args.push(spec.model.clone());
            }
            args
        } else {
            let mut args = vec![
                "exec".to_string(),
                "--json".to_string(),
                "--skip-git-repo-check".to_string(),
                "--ignore-user-config".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
            ];
            if !spec.model.is_empty() {
                args.push("--model".into());
                args.push(spec.model.clone());
            }
            args
        }
    }
}

/// Reduce a semver-ish version string to "major.minor".
pub fn version_band(version: &str) -> String {
    let v = version.trim().trim_start_matches('v');
    let parts: Vec<&str> = v.splitn(3, '.').collect();
    if parts.len() >= 2 {
        format!("{}.{}", parts[0], parts[1])
    } else {
        v.to_string()
    }
}

fn build_prompt(spec: &RunSpec) -> String {
    if spec.skill_installed {
        // Forced mode: explicitly direct the agent to use the installed skill so we
        // measure content quality, not trigger/activation behavior.
        format!(
            "Use the skill at .claude/skills/{}/SKILL.md for this task.\n\n{}",
            spec.skill_slug, spec.instruction
        )
    } else {
        spec.instruction.clone()
    }
}

fn install_skill_into_workspace(spec: &RunSpec) -> Result<(), String> {
    if spec.skill_content.trim().is_empty() {
        return Err("treatment arm requires skill content but none was provided".into());
    }
    let dir = spec
        .working_dir
        .join(".claude")
        .join("skills")
        .join(&spec.skill_slug);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("SKILL.md"), &spec.skill_content).map_err(|e| e.to_string())
}

// ---- usage parsing (pure, unit-tested directly) ----

/// Dispatch to the provider-specific parser.
pub fn parse_agent_usage(provider: &str, stdout: &[u8]) -> Result<RunOutcome, String> {
    if provider == "claude" {
        parse_claude_usage(stdout)
    } else {
        parse_codex_usage(stdout)
    }
}

#[derive(Deserialize, Default)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: i64,
    #[serde(default)]
    cache_creation_input_tokens: i64,
}

#[derive(Deserialize, Default)]
struct ClaudeResult {
    #[serde(default)]
    total_cost_usd: Option<f64>,
    #[serde(default)]
    num_turns: i64,
    #[serde(default)]
    duration_ms: i64,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    usage: ClaudeUsage,
}

fn parse_claude_usage(stdout: &[u8]) -> Result<RunOutcome, String> {
    let res: ClaudeResult = match serde_json::from_slice(stdout) {
        Ok(r) => r,
        Err(e) => {
            // If a stream slipped through, take the last JSON object line.
            match last_json_object(stdout) {
                Some(obj) => {
                    serde_json::from_slice(&obj).map_err(|_| format!("parse claude usage: {e}"))?
                }
                None => return Err(format!("parse claude usage: {e}")),
            }
        }
    };
    Ok(RunOutcome {
        input_tokens: res.usage.input_tokens,
        output_tokens: res.usage.output_tokens,
        cache_read: res.usage.cache_read_input_tokens,
        cache_write: res.usage.cache_creation_input_tokens,
        cost_usd: res.total_cost_usd,
        duration_ms: res.duration_ms,
        turns: res.num_turns,
        agent_error: res.is_error,
        trajectory_digest: String::new(),
    })
}

#[derive(Deserialize, Default)]
struct CodexUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    cached_input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    reasoning_output_tokens: i64,
}

#[derive(Deserialize, Default)]
struct CodexEvent {
    #[serde(default, rename = "type")]
    event_type: String,
    #[serde(default)]
    usage: CodexUsage,
}

fn parse_codex_usage(stdout: &[u8]) -> Result<RunOutcome, String> {
    let text = String::from_utf8_lossy(stdout);
    let mut out = RunOutcome::default();
    let mut found = false;
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            continue;
        }
        let Ok(evt): Result<CodexEvent, _> = serde_json::from_str(line) else {
            continue;
        };
        if evt.event_type == "turn.completed" {
            found = true;
            out.input_tokens += evt.usage.input_tokens;
            out.output_tokens += evt.usage.output_tokens + evt.usage.reasoning_output_tokens;
            out.cache_read += evt.usage.cached_input_tokens;
            out.turns += 1;
        }
    }
    if !found {
        return Err("parse codex usage: no turn.completed events".into());
    }
    Ok(out)
}

fn last_json_object(data: &[u8]) -> Option<Vec<u8>> {
    let text = String::from_utf8_lossy(data);
    for line in text.split('\n').rev() {
        let line = line.trim();
        if line.starts_with('{') && line.ends_with('}') {
            return Some(line.as_bytes().to_vec());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_claude_usage_works() {
        let stdout = br#"{"type":"result","total_cost_usd":0.1234,"num_turns":5,"duration_ms":8200,"is_error":false,"usage":{"input_tokens":1200,"output_tokens":800,"cache_read_input_tokens":5000,"cache_creation_input_tokens":300}}"#;
        let out = parse_claude_usage(stdout).unwrap();
        assert_eq!(out.input_tokens, 1200);
        assert_eq!(out.output_tokens, 800);
        assert_eq!(out.cache_read, 5000);
        assert_eq!(out.cache_write, 300);
        assert_eq!(out.cost_usd, Some(0.1234));
        assert_eq!(out.turns, 5);
    }

    #[test]
    fn parse_codex_usage_accumulates() {
        let stdout = b"{\"type\":\"thread.started\"}\n{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":500,\"cached_input_tokens\":100,\"output_tokens\":200,\"reasoning_output_tokens\":50}}\n{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":300,\"cached_input_tokens\":80,\"output_tokens\":150,\"reasoning_output_tokens\":25}}";
        let out = parse_codex_usage(stdout).unwrap();
        assert_eq!(out.input_tokens, 800);
        assert_eq!(out.output_tokens, 425);
        assert_eq!(out.cache_read, 180);
        assert_eq!(out.turns, 2);
    }

    #[test]
    fn parse_codex_usage_no_turns_errors() {
        assert!(parse_codex_usage(b"{\"type\":\"thread.started\"}").is_err());
    }

    #[test]
    fn version_band_works() {
        assert_eq!(version_band("2.1.174"), "2.1");
        assert_eq!(version_band("v2.1.176"), "2.1");
        assert_eq!(version_band("1.0"), "1.0");
        assert_eq!(version_band("3"), "3");
    }

    #[test]
    fn build_prompt_forced_vs_control() {
        let control = build_prompt(&RunSpec {
            working_dir: ".".into(),
            instruction: "fix".into(),
            model: String::new(),
            skill_installed: false,
            skill_content: String::new(),
            skill_slug: String::new(),
            timeout_sec: 0,
        });
        assert_eq!(control, "fix");
        let treatment = build_prompt(&RunSpec {
            working_dir: ".".into(),
            instruction: "fix".into(),
            model: String::new(),
            skill_installed: true,
            skill_content: String::new(),
            skill_slug: "owner/skill".into(),
            timeout_sec: 0,
        });
        assert_ne!(treatment, control);
        assert!(treatment.contains(".claude/skills/owner/skill/SKILL.md"));
    }
}
