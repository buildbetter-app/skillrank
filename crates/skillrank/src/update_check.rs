//! Startup update check — tell the user a newer skillrank exists, without ever
//! degrading the command they actually ran.
//!
//! Shape, and why it is this shape:
//!
//! * **No network on the hot path.** The check reads a small JSON cache under
//!   `~/.skillrank` and prints at most one line. The HTTP lookup that refreshes
//!   that cache runs *after* the command has produced its output, at most once
//!   per [`CACHE_TTL_SECS`], and with a hard [`REFRESH_TIMEOUT`] — so a slow,
//!   captive, or dead network can add at most a couple of seconds to a
//!   once-a-day invocation, and nothing at all to the rest.
//! * **Never changes the exit code, never fails the command.** Every error here
//!   — offline, DNS, rate limit, unwritable home, corrupt cache — is swallowed.
//!   A broken update check must not be able to break `skillrank install`.
//! * **stderr only.** stdout carries `--json` payloads that callers parse.
//! * **Notify by default; auto-apply is opt-in** via `SKILLRANK_AUTO_UPDATE=1`.
//!   This binary is invoked constantly inside agent loops and scripts. Swapping
//!   the executable underneath an unrelated command would change behaviour
//!   mid-session, with no prompt and no relationship to what the user asked for
//!   — the sort of surprise that turns "why did this break?" into an hour of
//!   confusion. So the default is one line of prose that makes upgrading a
//!   one-liner, and anyone who wants the machine to just handle it says so once
//!   with an env var.

use crate::update;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Check about once a day. Often enough that a client-side security fix reaches
/// people quickly; rare enough that it is not a per-invocation cost.
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;
/// Hard ceiling on the refresh request. The user is not waiting on this.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(2);
const CACHE_FILE: &str = "update-check.json";
/// Protocol- or long-lived commands, plus the updater itself. `mcp` speaks
/// JSON-RPC over stdio, `serve` is a daemon, and `update` already reports its
/// own version story.
const SKIPPED_SUBCOMMANDS: [&str; 4] = ["mcp", "serve", "update", "self-update"];

/// Run the check. Call this *after* the command has written its output, and
/// ignore it entirely when deciding the exit code.
pub fn run(args: &[String]) {
    check(&Context::from_env(args), env!("CARGO_PKG_VERSION"));
}

/// Everything the skip decision depends on, captured up front so the policy is
/// testable without mutating process-global environment state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub subcommand: Option<String>,
    pub ci: bool,
    pub opted_out: bool,
    pub auto_update: bool,
    pub stderr_is_terminal: bool,
}

impl Context {
    fn from_env(args: &[String]) -> Context {
        Context {
            subcommand: subcommand_of(args).map(str::to_string),
            ci: env_flag("CI"),
            opted_out: env_flag("SKILLRANK_NO_UPDATE_CHECK"),
            auto_update: env_flag("SKILLRANK_AUTO_UPDATE"),
            stderr_is_terminal: std::io::stderr().is_terminal(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Skip {
    /// A subcommand whose output or lifetime a notice would interfere with.
    Subcommand,
    /// `CI` is set: a version notice is log noise nobody acts on, and
    /// auto-applying on a build machine would be worse.
    Ci,
    /// `SKILLRANK_NO_UPDATE_CHECK` is set.
    OptedOut,
    /// stderr is a pipe or a file — keep captured logs clean.
    NotATerminal,
}

fn skip_reason(ctx: &Context) -> Option<Skip> {
    if ctx.opted_out {
        return Some(Skip::OptedOut);
    }
    if ctx.ci {
        return Some(Skip::Ci);
    }
    if !ctx.stderr_is_terminal {
        return Some(Skip::NotATerminal);
    }
    match ctx.subcommand.as_deref() {
        Some(sub) if SKIPPED_SUBCOMMANDS.contains(&sub) => Some(Skip::Subcommand),
        _ => None,
    }
}

fn check(ctx: &Context, current: &str) {
    if skip_reason(ctx).is_some() {
        return;
    }
    let Some(path) = cache_path() else {
        return;
    };
    let cached = read_cache(&path);
    let now = unix_now();
    let cached_newer = cached
        .as_ref()
        .map(|entry| entry.latest_version.as_str())
        .filter(|latest| update::is_newer(latest, current));

    // Hot path: everything up to here is one small file read, no network.
    if let Some(latest) = cached_newer {
        if !ctx.auto_update {
            eprintln!("{}", notice(current, latest));
        }
    }
    let cached_is_newer = cached_newer.is_some();

    let stale = cached
        .as_ref()
        .is_none_or(|entry| !is_fresh(entry.checked_at, now, CACHE_TTL_SECS));
    // In auto mode a known-newer version is confirmed against the registry
    // before anything is swapped, so a stale or yanked cache entry can never be
    // what drives a binary replacement.
    let needs_network = stale || (ctx.auto_update && cached_is_newer);
    if !needs_network {
        return;
    }

    let Ok(release) = update::latest_release(Some(REFRESH_TIMEOUT)) else {
        return;
    };
    let latest = release.version().to_string();
    write_cache(
        &path,
        &Entry {
            checked_at: now,
            latest_version: latest.clone(),
        },
    );
    if !update::is_newer(&latest, current) {
        return;
    }
    if ctx.auto_update {
        // Reuses `skillrank update`'s download, size-check, and atomic swap.
        // A failure is silent by design: the command the user ran already
        // succeeded, and the next invocation will simply try again.
        if update::apply(&release).is_ok() {
            eprintln!("skillrank updated {current} -> {latest} (SKILLRANK_AUTO_UPDATE=1).");
        }
    } else if !cached_is_newer {
        // We learned about this release just now, after paying for the request
        // anyway — say so immediately rather than staying quiet until the next
        // invocation reads the cache.
        eprintln!("{}", notice(current, &latest));
    }
}

/// One short line. Not a banner: this prints above whatever the user was
/// actually reading, so it earns exactly one line and has to carry both the
/// upgrade command and the way to stop having to run it.
fn notice(current: &str, latest: &str) -> String {
    format!(
        "skillrank {latest} available (you have {current}): run `skillrank update`, or set SKILLRANK_AUTO_UPDATE=1 to auto-apply."
    )
}

/// The subcommand the user invoked, mirroring `main::run`'s leading-`--` strip.
fn subcommand_of(args: &[String]) -> Option<&str> {
    let mut args = args.iter().map(String::as_str);
    match args.next()? {
        "--" => args.next(),
        first => Some(first),
    }
}

/// Env vars are opt-in switches: set and meaningfully true. `0`, `false`, and
/// empty read as unset so `CI=0` or `SKILLRANK_AUTO_UPDATE=` behave sanely.
fn env_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(value) => {
            let value = value.trim();
            !value.is_empty()
                && !value.eq_ignore_ascii_case("0")
                && !value.eq_ignore_ascii_case("false")
        }
        Err(_) => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    checked_at: u64,
    latest_version: String,
}

/// Fresh means "written in the past, within the TTL". A timestamp in the future
/// (clock skew, a hand-edited or tampered cache) counts as stale rather than
/// fresh-forever, so the check self-heals on the next run.
fn is_fresh(checked_at: u64, now: u64, ttl: u64) -> bool {
    checked_at <= now && now - checked_at < ttl
}

/// Tolerant on purpose: a cache we cannot understand is treated as no cache.
fn parse_cache(body: &str) -> Option<Entry> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let checked_at = value.get("checked_at")?.as_u64()?;
    let latest_version = value.get("latest_version")?.as_str()?.trim();
    if latest_version.is_empty() {
        return None;
    }
    Some(Entry {
        checked_at,
        latest_version: latest_version.to_string(),
    })
}

fn cache_path() -> Option<PathBuf> {
    skillrank_core::config::home()
        .ok()
        .map(|home| home.join(CACHE_FILE))
}

fn read_cache(path: &std::path::Path) -> Option<Entry> {
    parse_cache(&std::fs::read_to_string(path).ok()?)
}

/// Write via a temp file + rename so a concurrent reader never sees a half
/// written cache. Failures (read-only home, full disk) are ignored — the only
/// cost is checking again next time.
fn write_cache(path: &std::path::Path, entry: &Entry) {
    let body = serde_json::json!({
        "checked_at": entry.checked_at,
        "latest_version": entry.latest_version,
    })
    .to_string();
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    if std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(subcommand: Option<&str>) -> Context {
        Context {
            subcommand: subcommand.map(str::to_string),
            ci: false,
            opted_out: false,
            auto_update: false,
            stderr_is_terminal: true,
        }
    }

    #[test]
    fn checks_on_ordinary_subcommands() {
        for sub in [
            None,
            Some("install"),
            Some("list"),
            Some("search"),
            Some("eval"),
        ] {
            assert_eq!(skip_reason(&ctx(sub)), None, "expected a check for {sub:?}");
        }
    }

    #[test]
    fn skips_protocol_and_updater_subcommands() {
        for sub in ["mcp", "serve", "update", "self-update"] {
            assert_eq!(
                skip_reason(&ctx(Some(sub))),
                Some(Skip::Subcommand),
                "expected {sub} to skip"
            );
        }
    }

    #[test]
    fn skips_in_ci_and_when_opted_out_and_when_stderr_is_redirected() {
        let mut ci = ctx(Some("list"));
        ci.ci = true;
        assert_eq!(skip_reason(&ci), Some(Skip::Ci));

        let mut opted_out = ctx(Some("list"));
        opted_out.opted_out = true;
        assert_eq!(skip_reason(&opted_out), Some(Skip::OptedOut));

        let mut piped = ctx(Some("list"));
        piped.stderr_is_terminal = false;
        assert_eq!(skip_reason(&piped), Some(Skip::NotATerminal));
    }

    #[test]
    fn opt_out_beats_auto_update() {
        // Asking for silence must win over asking for automation, or there is
        // no way to fully disable the feature.
        let mut both = ctx(Some("list"));
        both.opted_out = true;
        both.auto_update = true;
        assert_eq!(skip_reason(&both), Some(Skip::OptedOut));
    }

    #[test]
    fn auto_update_alone_does_not_skip() {
        let mut auto = ctx(Some("list"));
        auto.auto_update = true;
        assert_eq!(skip_reason(&auto), None);
    }

    #[test]
    fn subcommand_is_read_the_way_main_dispatches() {
        let args = |args: &[&str]| args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(subcommand_of(&args(&["list", "--json"])), Some("list"));
        assert_eq!(subcommand_of(&args(&["--", "mcp"])), Some("mcp"));
        assert_eq!(subcommand_of(&args(&[])), None);
        assert_eq!(subcommand_of(&args(&["--"])), None);
        assert_eq!(subcommand_of(&args(&["--version"])), Some("--version"));
    }

    #[test]
    fn cache_is_fresh_only_inside_the_ttl() {
        let now = 1_730_000_000;
        assert!(is_fresh(now, now, CACHE_TTL_SECS));
        assert!(is_fresh(now - 1, now, CACHE_TTL_SECS));
        assert!(is_fresh(now - (CACHE_TTL_SECS - 1), now, CACHE_TTL_SECS));
        assert!(!is_fresh(now - CACHE_TTL_SECS, now, CACHE_TTL_SECS));
        assert!(!is_fresh(now - CACHE_TTL_SECS * 30, now, CACHE_TTL_SECS));
    }

    #[test]
    fn future_timestamps_are_stale_not_fresh_forever() {
        let now = 1_730_000_000;
        assert!(!is_fresh(now + 1, now, CACHE_TTL_SECS));
        assert!(!is_fresh(u64::MAX, now, CACHE_TTL_SECS));
        assert!(is_fresh(0, 0, CACHE_TTL_SECS));
    }

    #[test]
    fn parses_a_well_formed_cache() {
        let entry =
            parse_cache(r#"{"checked_at":1730000000,"latest_version":"0.2.0"}"#).expect("parsed");
        assert_eq!(entry.checked_at, 1_730_000_000);
        assert_eq!(entry.latest_version, "0.2.0");
    }

    #[test]
    fn ignores_extra_fields_and_trims_the_version() {
        let entry =
            parse_cache(r#"{"checked_at":1,"latest_version":" 0.3.0 ","note":"hi","n":[1,2]}"#)
                .expect("parsed");
        assert_eq!(entry.latest_version, "0.3.0");
    }

    #[test]
    fn treats_unusable_cache_contents_as_absent() {
        for body in [
            "",
            "   ",
            "not json",
            "{",
            "[]",
            "null",
            r#"{"checked_at":1730000000}"#,
            r#"{"latest_version":"0.2.0"}"#,
            r#"{"checked_at":"soon","latest_version":"0.2.0"}"#,
            r#"{"checked_at":-5,"latest_version":"0.2.0"}"#,
            r#"{"checked_at":1.5,"latest_version":"0.2.0"}"#,
            r#"{"checked_at":1730000000,"latest_version":""}"#,
            r#"{"checked_at":1730000000,"latest_version":42}"#,
            r#"{"checked_at":1730000000,"latest_version":null}"#,
        ] {
            assert_eq!(parse_cache(body), None, "expected {body:?} to be ignored");
        }
    }

    #[test]
    fn missing_or_unreadable_cache_file_is_not_an_error() {
        let missing = std::env::temp_dir().join("skillrank-update-check-does-not-exist.json");
        let _ = std::fs::remove_file(&missing);
        assert_eq!(read_cache(&missing), None);
        // A directory where the cache should be is just as survivable.
        assert_eq!(read_cache(&std::env::temp_dir()), None);
    }

    #[test]
    fn write_then_read_round_trips() {
        let path = std::env::temp_dir().join(format!(
            "skillrank-update-check-{}-{:?}.json",
            std::process::id(),
            std::thread::current().id()
        ));
        let entry = Entry {
            checked_at: 1_730_000_000,
            latest_version: "0.9.1".to_string(),
        };
        write_cache(&path, &entry);
        assert_eq!(read_cache(&path), Some(entry));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unwritable_cache_path_is_silently_tolerated() {
        let path = std::env::temp_dir()
            .join("skillrank-no-such-dir-9f3a")
            .join(CACHE_FILE);
        write_cache(
            &path,
            &Entry {
                checked_at: 1,
                latest_version: "0.2.0".to_string(),
            },
        );
        assert_eq!(read_cache(&path), None);
    }

    #[test]
    fn notice_is_one_actionable_line() {
        let line = notice("0.1.4", "0.2.0");
        assert_eq!(line.lines().count(), 1);
        assert!(line.contains("0.2.0") && line.contains("0.1.4"));
        assert!(line.contains("skillrank update"));
        assert!(line.contains("SKILLRANK_AUTO_UPDATE=1"));
    }
}
