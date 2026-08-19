//! skillrank — find, install, evaluate, and publish agent skills. Standalone and
//! open source; integrates with BuildBetter ZeroShot when installed.

mod commands;
mod eval;
mod flags;
mod managed;
mod mcp;
mod selfskill;
mod serve;
mod setup;
mod update;
mod update_check;

use flags::Flags;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(args.clone());
    // Runs after the command's own output, and cannot influence the exit code:
    // a stale-version notice is never worth failing an install over.
    update_check::run(&args);
    std::process::exit(code);
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
        "outdated" => commands::outdated(tail),
        "upgrade" => commands::upgrade(tail),
        "uninstall" | "remove" | "rm" => commands::uninstall(tail),
        "recommend" => commands::recommend(tail),
        "eval" => eval::run(tail),
        "skill" => selfskill::run(tail),
        "mcp" => mcp::run(tail),
        "setup" => setup::run(tail),
        "update" | "self-update" => update::run(tail),
        "serve" => serve::run(tail),
        other => {
            eprintln!("unknown skillrank subcommand {other:?}");
            print_usage();
            2
        }
    }
}

/// login obtains a registry token so publish/rate/review can authenticate. The
/// core CLI (search/install) never needs one.
///
/// With no flags this runs the GitHub device flow, which is the only path that
/// yields a *verified* account — the kind whose published results can
/// corroborate a skill. `--anonymous` mints an unverified account instead (one
/// keystroke, results stay self-reported), and `--token` remains for CI, where
/// no human is present to approve anything.
fn login(args: &[String]) -> i32 {
    let f = Flags::parse(args);

    let pasted = if !f.value("token").is_empty() {
        f.value("token").to_string()
    } else {
        std::env::var("SKILLRANK_TOKEN").unwrap_or_default()
    };
    if !pasted.trim().is_empty() {
        return match save_token(&pasted) {
            Ok(_) => {
                println!("Saved. You can now publish and review skills.");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        };
    }

    let client = commands::new_client(&f);
    if f.bool("anonymous") {
        return match client.provision_anonymous_token() {
            Ok(grant) => finish_login(&grant.token, "anonymous"),
            Err(e) => {
                eprintln!("error: could not create an anonymous account: {e}");
                1
            }
        };
    }
    device_login(&client)
}

/// Print the code, open the browser, and poll until the human finishes.
fn device_login(client: &skillrank_core::Client) -> i32 {
    let started = match client.start_device_authorization() {
        Ok(started) => started,
        Err(e) => {
            eprintln!("error: could not start GitHub sign-in: {e}");
            eprintln!("       `skillrank login --anonymous` publishes without an identity.");
            return 1;
        }
    };

    println!("To sign in, open:  {}", started.verification_uri);
    println!("and enter the code:  {}", started.user_code);
    // Best effort: the code and URL are already printed, so a headless box or a
    // missing opener costs nothing.
    open_in_browser(&started.verification_uri);
    println!("\nWaiting for approval…");

    // The registry asks for a pace and raises it when GitHub says slow_down;
    // honour whatever it last asked for rather than a fixed interval.
    let mut interval = started.interval.max(1);
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(if started.expires_in == 0 {
            900
        } else {
            started.expires_in
        });
    loop {
        if std::time::Instant::now() >= deadline {
            eprintln!(
                "error: the code expired before it was approved; run `skillrank login` again."
            );
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_secs(interval));
        match client.poll_device_token(&started.device_code) {
            Ok(skillrank_core::types::DevicePoll::Granted(grant)) => {
                let kind = if grant.kind.is_empty() {
                    "github"
                } else {
                    &grant.kind
                };
                return finish_login(&grant.token, kind);
            }
            Ok(skillrank_core::types::DevicePoll::Pending { interval: next }) => {
                interval = next.max(1);
            }
            Err(e) => {
                eprintln!("error: sign-in failed: {e}");
                return 1;
            }
        }
    }
}

fn finish_login(token: &str, kind: &str) -> i32 {
    match save_token(token) {
        Ok(_) => {
            if kind == "anonymous" {
                println!(
                    "Signed in anonymously. You can publish; results stay self-reported.\nRun `skillrank login` any time to upgrade to a verified account."
                );
            } else {
                println!(
                    "Signed in ({kind}, verified). Your published results can corroborate a skill."
                );
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "windows")]
    let opener = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let opener = "xdg-open";
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
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
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
    {
        println!("Authenticated via SKILLRANK_TOKEN.");
        return 0;
    }
    // Same precedence as the core's own token resolution. Checking only the env
    // var used to report "not signed in" to someone who had just run `login`,
    // because that writes the auth file rather than the environment.
    if saved_token().is_some() {
        println!("Signed in (token stored in ~/.skillrank/auth.json).");
    } else {
        println!("Not signed in (reads and local eval still work).");
    }
    0
}

fn saved_token() -> Option<String> {
    let path = skillrank_core::config::auth_path().ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let token = parsed.get("token")?.as_str()?.trim().to_string();
    (!token.is_empty()).then_some(token)
}

fn save_token(token: &str) -> std::io::Result<()> {
    let path = skillrank_core::config::auth_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        // `~/.skillrank` is a fixed path in a home directory that may be shared;
        // the directory listing alone should not be readable by other accounts.
        set_owner_only(parent, 0o700);
    }
    let body = serde_json::json!({ "token": token.trim() });
    std::fs::write(&path, serde_json::to_string_pretty(&body).unwrap())?;
    // A bearer token written with the default umask lands world-readable on a
    // typical box. Narrow it after the write, and on every write, so a file
    // created by an older build gets corrected too.
    set_owner_only(&path, 0o600);
    Ok(())
}

/// Best-effort permission narrowing; a no-op where the platform has no POSIX
/// mode. Failure is not fatal — refusing to save the token would be worse than
/// saving it with the umask's permissions.
fn set_owner_only(path: &std::path::Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
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
  outdated           Show installed skills that have a newer version.
  upgrade [<slug>]   Update installed skills to the latest (--all for every outdated one).
  uninstall <slug>   Remove an installed skill.
  recommend          Suggest skills for this repo's detected stack.
  eval <ref>         Run a local paired eval on your own agent; optionally publish.
  login              Sign in with GitHub (opens your browser) so published results
                     can corroborate a skill. --anonymous publishes without an
                     identity; --token <token> is for CI.
  logout             Forget the stored token.
  whoami             Show whether this machine is signed in.
  skill [--install]  Print, or install into .claude/skills, the SKILL.md that
                     teaches your agent (Claude Code/Codex) to use skillrank.
  setup              Register skillrank MCP, Skill, and /skillrank command with
                     Claude Code and Codex (one-time). --triggers=user-only
                     makes the Skill fire only when you ask about skills;
                     --force replaces files you have edited or deleted.
  update             Update this skillrank binary from the latest GitHub release
                     and refresh the installed Skill and command.
  mcp                Run as an MCP stdio server (invoked by the agent; not by you).
  serve [--port N]   Run a local registry server (seed catalog) so search/install
                     work with no hosted backend. Set SKILLRANK_API_URL to it.

Global flags:
  --json             Emit JSON.
  --api-base-url URL Override the registry API base URL.
"
    );
}
