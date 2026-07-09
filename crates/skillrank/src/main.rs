//! skillrank — find, install, evaluate, and publish agent skills. Standalone and
//! open source; integrates with BuildBetter ZeroShot when installed.

mod commands;
mod eval;
mod flags;
mod mcp;
mod selfskill;
mod serve;
mod setup;
mod update;

use flags::Flags;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(args));
}

fn run(mut args: Vec<String>) -> i32 {
    if args.first().map(|s| s == "--").unwrap_or(false) {
        args.remove(0);
    }
    if args.is_empty() {
        return dispatch(&[]);
    }
    match args[0].as_str() {
        "--version" | "-V" | "version" => {
            println!("skillrank {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "--help" | "-h" | "help" => dispatch(&["help".to_string()]),
        "login" => login(&args[1..]),
        "logout" => logout(),
        "whoami" => whoami(),
        _ => dispatch(&args),
    }
}

fn dispatch(args: &[String]) -> i32 {
    let Some(sub) = args.first() else {
        print_usage();
        return 0;
    };
    let tail = &args[1..];
    match sub.as_str() {
        "help" | "--help" | "-h" => {
            print_usage();
            0
        }
        "search" => commands::search(tail),
        "show" => commands::show(tail),
        "install" | "add" => commands::install(tail),
        "list" | "ls" => commands::list(tail),
        "uninstall" | "remove" | "rm" => commands::uninstall(tail),
        "recommend" => commands::recommend(tail),
        "eval" => eval::run(tail),
        "skill" => selfskill::run(tail),
        "mcp" => mcp::run(tail),
        "setup" => setup::run(tail),
        "update" | "upgrade" | "self-update" => update::run(tail),
        "serve" => serve::run(tail),
        other => {
            eprintln!("unknown skillrank subcommand {other:?}");
            print_usage();
            2
        }
    }
}

/// login stores a registry token so publish/rate/review can authenticate. The
/// core CLI (search/install) never needs it.
fn login(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let token = if !f.value("token").is_empty() {
        f.value("token").to_string()
    } else {
        std::env::var("SKILLRANK_TOKEN").unwrap_or_default()
    };
    if token.trim().is_empty() {
        println!("Publishing and reviewing require a registry token.");
        println!("Get one from your registry account, then run:");
        println!("  skillrank login --token <token>");
        println!("\n(Search, install, and local eval need no account.)");
        return 1;
    }
    match save_token(&token) {
        Ok(_) => {
            println!("Saved. You can now publish and review skills.");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn logout() -> i32 {
    match save_token("") {
        Ok(_) => {
            println!("Signed out.");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn whoami() -> i32 {
    if std::env::var("SKILLRANK_TOKEN")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        println!("Authenticated via SKILLRANK_TOKEN.");
    } else {
        println!("Not signed in (reads and local eval still work).");
    }
    0
}

fn save_token(token: &str) -> std::io::Result<()> {
    let path = skillrank_core::config::auth_path()?;
    let body = serde_json::json!({ "token": token.trim() });
    std::fs::write(path, serde_json::to_string_pretty(&body).unwrap())
}

fn print_usage() {
    print!(
        "skillrank — find, install, evaluate, and publish agent skills

Open source. Works on its own; the core (search, install) needs no account.
Integrates with BuildBetter ZeroShot when it is also installed
(equivalently available as `bb skills <command>`).

Usage:
  skillrank <command> [flags]

Commands:
  search <query>     Search the public skill registry.
  show <ref>         Show a skill's scores, security, and eval results.
  install <ref>      Install a skill into this repo (hash-verified).
  list               List installed skills and drift.
  uninstall <slug>   Remove an installed skill.
  recommend          Suggest skills for this repo's detected stack.
  eval <ref>         Run a local paired eval on your own agent; optionally publish.
  skill [--install]  Print, or install into .claude/skills, the SKILL.md that
                     teaches your agent (Claude Code/Codex) to use skillrank.
  setup              Register skillrank MCP, Skill, and /skillrank command with
                     Claude Code and Codex (one-time).
  update             Update this skillrank binary from the latest GitHub release.
  mcp                Run as an MCP stdio server (invoked by the agent; not by you).
  serve [--port N]   Run a local registry server (seed catalog) so search/install
                     work with no hosted backend. Set SKILLRANK_API_URL to it.

Global flags:
  --json             Emit JSON.
  --api-base-url URL Override the registry API base URL.
"
    );
}
