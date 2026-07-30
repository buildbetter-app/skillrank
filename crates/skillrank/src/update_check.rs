//! Startup update check — tell the user a newer skillrank exists, without ever
//! degrading the command they actually ran.
//!
//! Shape, and why it is this shape:
//!
//! * **No network on the hot path.** The check reads a small JSON cache under
//!   `~/.skillrank` and prints at most one line. The HTTP lookup that refreshes
//!   that cache runs *after* the command has produced its output and at most
//!   once per [`CACHE_TTL_SECS`] — including in auto mode, which is not a
//!   licence to hit the API on every run — with [`REFRESH_TIMEOUT`] bounding
//!   connect and transfer. A *failed* attempt is remembered too
//!   ([`RETRY_BACKOFF_SECS`]), so an offline machine stops paying that timeout
//!   on every single invocation.
//! * **One line per TTL, not one line per run.** The same TTL rate-limits the
//!   print. Agent harnesses allocate a PTY, so the not-a-terminal skip does not
//!   spare them; without this the notice lands in every tool result.
//! * **Never changes the exit code, never fails the command.** Errors here —
//!   offline, DNS, rate limit, unwritable home, corrupt cache — are swallowed.
//!   A broken update check must not be able to break `skillrank install`. The
//!   deliberate exception is a failed *auto-apply*, which is printed: see
//!   [`refresh`].
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

/// Check, and say something, about once a day. Often enough that a client-side
/// security fix reaches people quickly; rare enough that it is neither a
/// per-invocation cost nor a per-invocation line of output.
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;
/// After a failed lookup, wait this long before spending another
/// [`REFRESH_TIMEOUT`]. Shorter than the TTL because the answer is still
/// unknown; long enough that a laptop in a tunnel is not retrying constantly.
const RETRY_BACKOFF_SECS: u64 = 60 * 60;
/// Ceiling on the refresh request — connect *and* transfer, with the DNS caveat
/// documented on `update`'s agent. The user is not waiting on this.
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
    let before = read_cache(&path);
    let mut entry = before.clone().unwrap_or_default();
    let now = unix_now();

    // Hot path: one small file read and at most one line. No network.
    maybe_notify(&mut entry, ctx, current, now);
    if needs_refresh(&entry, now) {
        refresh(&mut entry, ctx, current, now);
    }
    // One write, and only when something actually changed — including when the
    // only news is that the lookup failed.
    if before.as_ref() != Some(&entry) {
        write_cache(&path, &entry);
    }
}

/// Whether the cached answer has aged out *and* we are not inside the backoff
/// from a failed attempt.
///
/// The second half is the whole negative cache: a failure never writes a fresh
/// `checked_at`, so without it an offline machine, a rate-limited one, or one
/// behind a captive portal re-attempts — and re-pays [`REFRESH_TIMEOUT`] — on
/// every invocation, forever. The first half is unconditional: auto mode has no
/// business hitting the API more often than notify mode does.
fn needs_refresh(entry: &Entry, now: u64) -> bool {
    !is_fresh(entry.checked_at, now, CACHE_TTL_SECS)
        && !is_fresh(entry.last_attempt_at, now, RETRY_BACKOFF_SECS)
}

/// Whether to print the notice now. Separate from the printing so the policy is
/// testable without capturing stderr.
fn should_notify(entry: &Entry, ctx: &Context, current: &str, now: u64) -> bool {
    if !update::is_newer(&entry.latest_version, current) {
        return false;
    }
    // Auto mode speaks for itself — unless applying this exact version already
    // failed on this machine, in which case it will never be retried and the
    // user needs the manual instruction like everyone else.
    if ctx.auto_update && entry.failed_version != entry.latest_version {
        return false;
    }
    // Rate-limited by the same TTL as the network refresh: this line prints on
    // top of whatever the user was actually reading.
    !is_fresh(entry.notified_at, now, CACHE_TTL_SECS)
}

fn maybe_notify(entry: &mut Entry, ctx: &Context, current: &str, now: u64) {
    if !should_notify(entry, ctx, current, now) {
        return;
    }
    eprintln!("{}", notice(current, &entry.latest_version));
    entry.notified_at = now;
}

/// Whether auto mode should replace the binary with `latest`. A version whose
/// apply already failed here is never retried: the usual cause is a
/// root-owned install directory, which fails identically every time.
fn should_auto_apply(entry: &Entry, ctx: &Context, latest: &str, current: &str) -> bool {
    ctx.auto_update && update::is_newer(latest, current) && entry.failed_version != latest
}

/// The network half: look up the latest release, fold the result into `entry`,
/// and either apply it (auto mode) or say it exists.
///
/// The attempt is recorded before it can fail, and an auto-apply failure is
/// *printed* rather than swallowed. That is the one error in this file the user
/// has to see: auto mode exists because they stopped watching, so a silent
/// failure means an unattended machine sits on an old binary indefinitely.
fn refresh(entry: &mut Entry, ctx: &Context, current: &str, now: u64) {
    entry.last_attempt_at = now;
    let Ok(release) = update::latest_release(Some(REFRESH_TIMEOUT)) else {
        return;
    };
    let latest = release.version().to_string();
    record_lookup(entry, &latest, now);

    if should_auto_apply(entry, ctx, &latest, current) {
        // Reuses `skillrank update`'s download, checksum verification, and
        // atomic swap — there is one updater, not two.
        match update::apply(&release, update::DOWNLOAD_TIMEOUT) {
            Ok(()) => {
                eprintln!("skillrank updated {current} -> {latest} (SKILLRANK_AUTO_UPDATE=1).")
            }
            Err(message) => {
                eprintln!("{message}");
                eprintln!(
                    "skillrank {latest} was not applied and will not be retried automatically; run `skillrank update`."
                );
                entry.failed_version = latest;
            }
        }
        return;
    }
    // We learned about this release just now, having paid for the request
    // anyway — say so immediately rather than staying quiet until tomorrow.
    // (A no-op when the hot path above already printed it this run.)
    maybe_notify(entry, ctx, current, now);
}

/// Fold a successful lookup into the cache.
fn record_lookup(entry: &mut Entry, latest: &str, now: u64) {
    entry.checked_at = now;
    entry.latest_version = latest.to_string();
    if entry.failed_version != latest {
        // A different release is different bytes; an earlier failure says
        // nothing about it.
        entry.failed_version.clear();
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

/// The cache is this feature's entire memory: what we last learned, when we
/// last managed to ask, when we last said anything, and what we already failed
/// to apply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Entry {
    /// Last *successful* lookup. 0 when there has never been one.
    checked_at: u64,
    latest_version: String,
    /// Last lookup *attempt*, successful or not — the negative cache.
    last_attempt_at: u64,
    /// A version whose auto-apply already failed here. Not retried.
    failed_version: String,
    /// When the notice was last printed.
    notified_at: u64,
}

/// Fresh means "written in the past, within the TTL". A timestamp in the future
/// (clock skew, a hand-edited or tampered cache) counts as stale rather than
/// fresh-forever, so the check self-heals on the next run.
fn is_fresh(checked_at: u64, now: u64, ttl: u64) -> bool {
    checked_at <= now && now - checked_at < ttl
}

/// Tolerant on purpose: a cache we cannot understand is treated as no cache.
/// Missing fields default (an older cache has no `notified_at`), but a field
/// that is *present and the wrong type* invalidates the whole entry — rechecking
/// costs one request, while trusting a corrupt "nothing new here" hides a
/// release for a day.
fn parse_cache(body: &str) -> Option<Entry> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let object = value.as_object()?;
    let number = |key: &str| match object.get(key) {
        None | Some(serde_json::Value::Null) => Some(0),
        Some(value) => value.as_u64(),
    };
    let text = |key: &str| match object.get(key) {
        None | Some(serde_json::Value::Null) => Some(String::new()),
        Some(value) => value.as_str().map(|s| s.trim().to_string()),
    };

    let checked_at = number("checked_at")?;
    let notified_at = number("notified_at")?;
    let latest_version = text("latest_version")?;
    let failed_version = text("failed_version")?;
    // Caches written before the negative cache existed only have `checked_at`,
    // and a successful check is also an attempt.
    let last_attempt_at = match object.get("last_attempt_at") {
        None | Some(serde_json::Value::Null) => checked_at,
        Some(value) => value.as_u64()?,
    };

    let completed_a_check = checked_at > 0;
    let knows_a_version = !latest_version.is_empty();
    if completed_a_check != knows_a_version {
        // Half a record: a completed check with nothing to show for it, or a
        // version with no timestamp to age it out.
        return None;
    }
    if !completed_a_check && last_attempt_at == 0 && notified_at == 0 {
        // No timestamps at all — nothing worth remembering.
        return None;
    }
    Some(Entry {
        checked_at,
        latest_version,
        last_attempt_at,
        failed_version,
        notified_at,
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
        "last_attempt_at": entry.last_attempt_at,
        "failed_version": entry.failed_version,
        "notified_at": entry.notified_at,
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
            last_attempt_at: 1_730_000_500,
            failed_version: "0.9.1".to_string(),
            notified_at: 1_730_000_100,
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
                ..Entry::default()
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

    const NOW: u64 = 1_730_000_000;

    fn known(latest: &str, checked_at: u64) -> Entry {
        Entry {
            checked_at,
            latest_version: latest.to_string(),
            last_attempt_at: checked_at,
            ..Entry::default()
        }
    }

    fn auto(subcommand: Option<&str>) -> Context {
        Context {
            auto_update: true,
            ..ctx(subcommand)
        }
    }

    #[test]
    fn auto_mode_respects_the_cache_ttl() {
        // Regression: `needs_network` used to be
        // `stale || (auto_update && cached_is_newer)`, so a ten-second-old
        // cache plus SKILLRANK_AUTO_UPDATE=1 hit the GitHub API on every single
        // invocation.
        // The mode is no longer an input to the decision at all — see the
        // signature — so the tempting case proves it: auto mode would apply
        // this exact cached version, and still does not go to the network.
        let fresh = known("0.2.0", NOW - 10);
        assert!(!needs_refresh(&fresh, NOW), "a fresh cache needs no lookup");
        assert!(should_auto_apply(
            &fresh,
            &auto(Some("list")),
            "0.2.0",
            "0.1.4"
        ));

        // The TTL still expires, and no cache still means look it up.
        let stale = known("0.2.0", NOW - CACHE_TTL_SECS);
        assert!(needs_refresh(&stale, NOW));
        assert!(needs_refresh(&Entry::default(), NOW), "no cache -> look up");
    }

    #[test]
    fn a_failed_lookup_is_cached_and_backed_off() {
        // Regression: the cache was only written on success, so every
        // invocation on an offline machine re-attempted and re-paid the
        // refresh timeout.
        let mut entry = Entry::default();
        assert!(needs_refresh(&entry, NOW));

        // What `refresh` records before the request can fail.
        entry.last_attempt_at = NOW;
        assert!(
            !needs_refresh(&entry, NOW + 1),
            "a failed attempt must not be retried immediately"
        );
        assert!(!needs_refresh(&entry, NOW + RETRY_BACKOFF_SECS - 1));
        assert!(
            needs_refresh(&entry, NOW + RETRY_BACKOFF_SECS),
            "the backoff has to expire"
        );

        // And it survives a round trip: a failure-only entry is a valid cache.
        let path = std::env::temp_dir().join(format!(
            "skillrank-update-check-failure-{}-{:?}.json",
            std::process::id(),
            std::thread::current().id()
        ));
        write_cache(&path, &entry);
        assert_eq!(read_cache(&path), Some(entry));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_successful_lookup_clears_the_backoff() {
        let mut entry = Entry {
            last_attempt_at: NOW - 60,
            ..Entry::default()
        };
        record_lookup(&mut entry, "0.2.0", NOW);
        assert_eq!(entry.checked_at, NOW);
        assert_eq!(entry.latest_version, "0.2.0");
        assert!(!needs_refresh(&entry, NOW + CACHE_TTL_SECS - 1));
        assert!(needs_refresh(&entry, NOW + CACHE_TTL_SECS));
    }

    #[test]
    fn the_notice_respects_its_own_ttl() {
        // Regression: the TTL rate-limited the network refresh but not the
        // print, so the line appeared above every command's output — and agent
        // harnesses allocate a PTY, so the not-a-terminal skip does not save
        // them from it.
        let mut entry = known("0.2.0", NOW);
        let ctx = ctx(Some("list"));
        assert!(should_notify(&entry, &ctx, "0.1.4", NOW));

        maybe_notify(&mut entry, &ctx, "0.1.4", NOW);
        assert_eq!(entry.notified_at, NOW);
        assert!(
            !should_notify(&entry, &ctx, "0.1.4", NOW + 1),
            "the second invocation of the day stays quiet"
        );
        assert!(!should_notify(
            &entry,
            &ctx,
            "0.1.4",
            NOW + CACHE_TTL_SECS - 1
        ));
        assert!(
            should_notify(&entry, &ctx, "0.1.4", NOW + CACHE_TTL_SECS),
            "a day later it is worth saying again"
        );
        // Nothing to say when the cached version is not newer.
        assert!(!should_notify(&entry, &ctx, "0.2.0", NOW + CACHE_TTL_SECS));
    }

    #[test]
    fn auto_mode_stays_quiet_until_it_cannot_apply() {
        let entry = known("0.2.0", NOW);
        assert!(
            !should_notify(&entry, &auto(Some("list")), "0.1.4", NOW),
            "auto mode applies instead of narrating"
        );

        // Once applying 0.2.0 has failed here it will never be retried, so the
        // user gets the manual instruction like everyone else — rather than the
        // silence that let an unattended machine sit on an old binary forever.
        let failed = Entry {
            failed_version: "0.2.0".to_string(),
            ..entry
        };
        assert!(should_notify(&failed, &auto(Some("list")), "0.1.4", NOW));
    }

    #[test]
    fn a_failed_auto_apply_is_not_retried() {
        let mut entry = known("0.2.0", NOW);
        let auto = auto(Some("list"));
        assert!(should_auto_apply(&entry, &auto, "0.2.0", "0.1.4"));

        // What `refresh` records when `update::apply` returns Err.
        entry.failed_version = "0.2.0".to_string();
        assert!(
            !should_auto_apply(&entry, &auto, "0.2.0", "0.1.4"),
            "the same failure must not repeat every day"
        );

        // A newer release is different bytes, so the old failure is discarded.
        record_lookup(&mut entry, "0.3.0", NOW + CACHE_TTL_SECS);
        assert_eq!(entry.failed_version, "");
        assert!(should_auto_apply(&entry, &auto, "0.3.0", "0.1.4"));

        // Notify mode never applies, whatever the cache says.
        assert!(!should_auto_apply(
            &entry,
            &ctx(Some("list")),
            "0.3.0",
            "0.1.4"
        ));
        // Neither does auto mode when there is nothing newer.
        assert!(!should_auto_apply(&entry, &auto, "0.1.4", "0.1.4"));
    }

    #[test]
    fn parses_the_negative_and_notified_fields() {
        let entry = parse_cache(
            r#"{"checked_at":1730000000,"latest_version":"0.2.0","last_attempt_at":1730000600,"failed_version":"0.2.0","notified_at":1730000300}"#,
        )
        .expect("parsed");
        assert_eq!(entry.last_attempt_at, 1_730_000_600);
        assert_eq!(entry.failed_version, "0.2.0");
        assert_eq!(entry.notified_at, 1_730_000_300);

        // A failure-only entry: no successful check yet, just an attempt.
        let attempt_only = parse_cache(r#"{"last_attempt_at":1730000000}"#).expect("parsed");
        assert_eq!(attempt_only.last_attempt_at, 1_730_000_000);
        assert_eq!(attempt_only.checked_at, 0);
        assert_eq!(attempt_only.latest_version, "");

        // A cache written by an older build has neither field; a successful
        // check is also an attempt.
        let legacy =
            parse_cache(r#"{"checked_at":1730000000,"latest_version":"0.2.0"}"#).expect("parsed");
        assert_eq!(legacy.last_attempt_at, 1_730_000_000);
        assert_eq!(legacy.notified_at, 0);
    }

    #[test]
    fn treats_wrongly_typed_new_fields_as_absent_cache() {
        for body in [
            r#"{"checked_at":1730000000,"latest_version":"0.2.0","last_attempt_at":"soon"}"#,
            r#"{"checked_at":1730000000,"latest_version":"0.2.0","notified_at":-1}"#,
            r#"{"checked_at":1730000000,"latest_version":"0.2.0","failed_version":7}"#,
        ] {
            assert_eq!(parse_cache(body), None, "expected {body:?} to be ignored");
        }
    }
}
