//! `skillrank mcp` — a Model Context Protocol stdio server. Once registered
//! (see `skillrank setup`), the agent gets first-class tools — skill_search,
//! skill_recommend, skill_show, skill_install, skill_list — so "find me a skill
//! for playwright" just works, in the agent's own tool vocabulary.
//!
//! Transport: newline-delimited JSON-RPC 2.0 over stdio. Protocol JSON goes to
//! stdout ONLY; everything else to stderr.

use crate::flags::Flags;
use serde_json::{json, Value};
use skillrank_core as core;
use std::io::{BufRead, Write};

pub fn run(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let override_url = f.value("api-base-url");
    let client = core::Client::new(if override_url.is_empty() {
        None
    } else {
        Some(override_url)
    });
    let server = McpServer { client };

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        server.handle_line(&line, &mut out);
    }
    0
}

pub struct McpServer {
    pub client: core::Client,
}

impl McpServer {
    pub fn handle_line(&self, line: &str, out: &mut impl Write) {
        let Ok(req): Result<Value, _> = serde_json::from_str(line) else {
            return; // ignore unparseable input
        };
        let id = req.get("id").cloned();
        let is_notification = matches!(id, None | Some(Value::Null));
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => self.reply(out, id, self.initialize_result(&params)),
            "notifications/initialized" | "initialized" => {}
            "ping" => self.reply(out, id, json!({})),
            "tools/list" => self.reply(out, id, json!({ "tools": tool_definitions() })),
            "tools/call" => self.reply(out, id, self.call_tool(&params)),
            _ => {
                if !is_notification {
                    self.reply_error(out, id, -32601, &format!("method not found: {method}"));
                }
            }
        }
    }

    fn initialize_result(&self, params: &Value) -> Value {
        let protocol_version = params
            .get("protocolVersion")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("2025-06-18");
        json!({
            "protocolVersion": protocol_version,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "skillrank", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "skillrank finds, installs, and evaluates agent skills. Use skill_search or skill_recommend to find skills, skill_show to inspect one, and skill_install to add it to this repo (it becomes available to the agent automatically)."
        })
    }

    fn reply(&self, out: &mut impl Write, id: Option<Value>, result: Value) {
        let Some(id) = id else { return }; // notification: no reply
        if id.is_null() {
            return;
        }
        let resp = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        let _ = writeln!(out, "{resp}");
        let _ = out.flush();
    }

    fn reply_error(&self, out: &mut impl Write, id: Option<Value>, code: i64, message: &str) {
        let id = id.unwrap_or(Value::Null);
        let resp =
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } });
        let _ = writeln!(out, "{resp}");
        let _ = out.flush();
    }

    fn call_tool(&self, params: &Value) -> Value {
        let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let args = params.get("arguments").cloned().unwrap_or(Value::Null);
        match name {
            "skill_search" => self.tool_search(&args),
            "skill_show" => self.tool_show(&args),
            "skill_recommend" => self.tool_recommend(&args),
            "skill_install" => self.tool_install(&args),
            "skill_list" => self.tool_list(&args),
            other => tool_text(&format!("unknown tool: {other}"), true),
        }
    }

    fn tool_search(&self, args: &Value) -> Value {
        let query = str_arg(args, "query");
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
        let resp = match self.client.search(&core::SearchOptions {
            query: query.clone(),
            stack: str_arg(args, "stack"),
            agent: str_arg(args, "agent"),
            category: str_arg(args, "category"),
            limit,
            ..Default::default()
        }) {
            Ok(r) => r,
            Err(e) => return tool_text(&format!("search failed: {e}"), true),
        };
        if resp.items.is_empty() {
            return tool_text(&format!("No skills matched \"{query}\"."), false);
        }
        let mut b = format!("{} skill(s) for \"{query}\":\n", resp.items.len());
        for item in &resp.items {
            b.push_str(&format!("- {} (scan: {})", item.slug, item.scan_tier));
            if !item.stacks.is_empty() {
                b.push_str(&format!(" [{}]", item.stacks.join(",")));
            }
            if !item.summary.is_empty() {
                b.push_str(&format!(" — {}", item.summary));
            }
            b.push('\n');
        }
        b.push_str("\nInstall one with the skill_install tool (ref = the slug).");
        tool_text(&b, false)
    }

    fn tool_show(&self, args: &Value) -> Value {
        let slug = str_arg(args, "slug");
        if slug.trim().is_empty() {
            return tool_text("slug is required", true);
        }
        match self.client.show(&slug) {
            Ok(detail) => tool_text(
                &serde_json::to_string_pretty(&detail).unwrap_or_default(),
                false,
            ),
            Err(e) => tool_text(&format!("show failed: {e}"), true),
        }
    }

    fn tool_recommend(&self, args: &Value) -> Value {
        let repo_root = core::repo_root(&str_arg(args, "cwd"));
        let detected = core::detect_stack(&repo_root);
        if detected.stacks.is_empty() {
            return tool_text(
                "Could not detect a stack in this repo. Use skill_search with a query instead.",
                false,
            );
        }
        let mut b = format!("Detected stack: {}\n", detected.stacks.join(", "));
        let mut seen = std::collections::HashSet::new();
        let mut found = 0;
        for stack in &detected.stacks {
            if let Ok(resp) = self.client.search(&core::SearchOptions {
                stack: stack.clone(),
                sort: "signals".into(),
                limit: 5,
                ..Default::default()
            }) {
                for item in resp.items {
                    if seen.insert(item.slug.clone()) {
                        found += 1;
                        b.push_str(&format!(
                            "- {} (scan: {}) — {}\n",
                            item.slug, item.scan_tier, item.summary
                        ));
                    }
                }
            }
        }
        if found == 0 {
            b.push_str("No matching skills in the registry yet.\n");
        }
        tool_text(&b, false)
    }

    fn tool_install(&self, args: &Value) -> Value {
        let reference = str_arg(args, "ref");
        if reference.trim().is_empty() {
            return tool_text("ref (skill slug) is required", true);
        }
        let repo_root = core::repo_root(&str_arg(args, "cwd"));
        let resolved = match self.client.resolve(&reference) {
            Ok(r) => r,
            Err(e) => return tool_text(&format!("resolve failed: {e}"), true),
        };
        let yes = args.get("yes").and_then(|v| v.as_bool()).unwrap_or(false);
        if !core::safe_scan_tier(resolved.scan_tier) && !yes {
            return tool_text(
                &format!(
                    "{} has scan tier {:?} (not verified safe). Ask the user to confirm, then call skill_install again with yes=true.",
                    resolved.slug, resolved.scan_tier
                ),
                true,
            );
        }
        match self.client.install(&core::InstallOptions {
            reference,
            repo_root,
            surface_override: str_arg(args, "surface"),
            prefix: true,
            now_rfc3339: None,
        }) {
            Ok(result) if result.already_exact => tool_text(
                &format!(
                    "{} is already installed at {} (up to date).",
                    result.slug, result.skill_path
                ),
                false,
            ),
            Ok(result) => tool_text(
                &format!(
                    "Installed {} → {} (scan: {}). It is now available to the agent in this repo automatically.",
                    result.slug, result.skill_path, result.scan_tier
                ),
                false,
            ),
            Err(e) => tool_text(&format!("install failed: {e}"), true),
        }
    }

    fn tool_list(&self, args: &Value) -> Value {
        let repo_root = core::repo_root(&str_arg(args, "cwd"));
        match core::list_installed(&repo_root) {
            Ok(rows) if rows.is_empty() => {
                tool_text("No registry-installed skills in this repo.", false)
            }
            Ok(rows) => {
                let mut b = String::new();
                for r in rows {
                    b.push_str(&format!("- {} [{}] {}\n", r.slug, r.state, r.skill_path));
                }
                tool_text(&b, false)
            }
            Err(e) => tool_text(&format!("list failed: {e}"), true),
        }
    }
}

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn tool_text(text: &str, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

/// The tools/list payload: the agent's new vocabulary.
pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "skill_search",
            "description": "Search the public skill registry for agent skills. Use when the user asks to find a skill for something (e.g. 'find me a skill for playwright').",
            "inputSchema": {
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": { "type": "string", "description": "What to search for, e.g. 'playwright' or 'react performance'." },
                    "stack": { "type": "string", "description": "Optional stack filter, e.g. nextjs, fastapi, go, playwright." },
                    "agent": { "type": "string", "description": "Optional agent filter, e.g. claude, codex." },
                    "category": { "type": "string", "description": "Optional category filter." },
                    "limit": { "type": "integer", "description": "Max results (default 15)." }
                }
            }
        },
        {
            "name": "skill_recommend",
            "description": "Recommend skills for the current repository by detecting its stack. Use when the user asks 'what skills should I use here'.",
            "inputSchema": {
                "type": "object",
                "properties": { "cwd": { "type": "string", "description": "Repo directory to inspect (default: current working directory)." } }
            }
        },
        {
            "name": "skill_show",
            "description": "Show a skill's details, security scan tier, and eval results by trust tier. Use to evaluate whether a skill is worth installing.",
            "inputSchema": {
                "type": "object",
                "required": ["slug"],
                "properties": { "slug": { "type": "string", "description": "The skill slug, e.g. owner/skill." } }
            }
        },
        {
            "name": "skill_install",
            "description": "Install a skill into this repository (hash-verified). It becomes available to the agent automatically. If the scan tier is unsafe, the tool asks for confirmation; re-call with yes=true after the user agrees.",
            "inputSchema": {
                "type": "object",
                "required": ["ref"],
                "properties": {
                    "ref": { "type": "string", "description": "Skill slug (optionally slug@version)." },
                    "surface": { "type": "string", "description": "Optional skill surface dir, e.g. .claude/skills or .agents/skills." },
                    "cwd": { "type": "string", "description": "Repo directory (default: current working directory)." },
                    "yes": { "type": "boolean", "description": "Confirm install despite an unsafe scan tier." }
                }
            }
        },
        {
            "name": "skill_list",
            "description": "List skills installed in this repo via skillrank, including drift (modified/removed).",
            "inputSchema": {
                "type": "object",
                "properties": { "cwd": { "type": "string", "description": "Repo directory (default: current working directory)." } }
            }
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> McpServer {
        McpServer {
            client: skillrank_core::Client::new(Some("http://127.0.0.1:1")),
        }
    }

    #[test]
    fn tool_definitions_expose_five_tools() {
        let defs = tool_definitions();
        let names: Vec<&str> = defs
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for want in [
            "skill_search",
            "skill_recommend",
            "skill_show",
            "skill_install",
            "skill_list",
        ] {
            assert!(names.contains(&want), "missing {want}");
        }
    }

    #[test]
    fn initialize_echoes_protocol_version() {
        let s = server();
        let mut out: Vec<u8> = Vec::new();
        s.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#, &mut out);
        let resp: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(resp["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(resp["result"]["serverInfo"]["name"], "skillrank");
    }

    #[test]
    fn notification_produces_no_response() {
        let s = server();
        let mut out: Vec<u8> = Vec::new();
        s.handle_line(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            &mut out,
        );
        assert!(out.is_empty(), "notification must not reply");
    }
}
