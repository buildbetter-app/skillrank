//! `skillrank update` — replace the current binary with the latest GitHub
//! release asset for this platform, then bring the files that binary owns on
//! this machine (the agent Skill and the `/skillrank` command) up to the text
//! it ships.
//!
//! The refresh exists because `setup` runs exactly once, from `install.sh`.
//! Without it, an install made in October keeps October's Skill description
//! forever no matter how many times the binary is updated — which means a
//! change to that description reaches nobody who already has skillrank. See
//! `docs/agent-initiated-skill-discovery-spec.md` (R7).
//!
//! The release lookup ([`latest_release`]) and the download+verify+swap
//! ([`apply`]) are also what the startup update check in [`crate::update_check`]
//! uses, so there is exactly one updater in this binary.
//!
//! "Verify" is load-bearing and means one specific thing: every release
//! publishes `<asset>.sha256` next to the binary
//! (`.github/workflows/release.yml`), and nothing is written over the running
//! executable until the downloaded bytes hash to exactly that digest. Failure
//! to *fetch* the checksum is a refusal too, not a warning — the same
//! fail-closed rule `install.sh` applies, and the promise `SECURITY.md` makes.

use crate::flags::Flags;
use crate::managed;
use serde_json::Value;
use skillrank_core::hash::sha256_hex;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/buildbetter-app/skillrank/releases/latest";
/// The only prefix an update may ever be downloaded from. Asserted against the
/// formatted URL, so no release tag can move the download somewhere else.
const RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/buildbetter-app/skillrank/releases/download/";
const USER_AGENT: &str = "skillrank";
const DOWNLOAD_MIN_BYTES: usize = 100 * 1024;
/// Hard cap on a downloaded asset. Release binaries are a few MB; this leaves
/// room to grow while keeping a hostile or misconfigured endpoint from
/// streaming the process out of memory.
const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;
/// A `.sha256` file is 64 hex characters and maybe a filename. Anything larger
/// is not a checksum.
const MAX_CHECKSUM_BYTES: u64 = 4 * 1024;
/// Longest release tag we will accept. Real tags are `v0.1.4`.
const MAX_TAG_LEN: usize = 64;
/// Ceiling on the download half of an update. Generous — a few MB over a slow
/// link — but finite, so a stalled transfer cannot wedge the process. Bounds
/// both `skillrank update` and the auto-apply path.
pub(crate) const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

pub fn run(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let current = env!("CARGO_PKG_VERSION");

    let release = match latest_release(None) {
        Ok(release) => release,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let latest = release.version();

    if !is_newer(latest, current) {
        if f.bool("check") {
            // `--check` answers a question; it does not change anything.
            println!("up to date");
        } else {
            println!("skillrank {current} is already up to date.");
            // Refresh here too, and not only after a swap. A swap replaces the
            // binary underneath a process that is still running the *old*
            // code, so the refresh it performs writes the old text — an
            // install upgrading into this feature would otherwise need a
            // manual `setup` to ever see the new Skill. Refreshing on the
            // already-current path means one more ordinary `skillrank update`
            // converges it, with no new command to learn.
            refresh_managed_files();
        }
        return 0;
    }

    if f.bool("check") {
        println!("update available: {latest} (current {current})");
        return 0;
    }

    let latest = latest.to_string();
    match apply(&release, DOWNLOAD_TIMEOUT) {
        Ok(()) => {
            println!("Updated skillrank {current} -> {latest}");
            refresh_managed_files();
            0
        }
        Err(message) => {
            eprintln!("{message}");
            1
        }
    }
}

/// Bring the installed Skill and `/skillrank` command up to the text this
/// running binary embeds.
///
/// Three things it deliberately does not do, because by the time this runs the
/// user's actual request has already succeeded and none of them is worth
/// turning that into a failure:
///
/// * **It never fails.** Every error is reported on its own line and swallowed;
///   the caller's exit code is decided by the update itself.
/// * **It never installs.** Only files that already exist are refreshed. A
///   machine that opted out of `setup` (or deleted the Skill) does not grow one
///   because it updated the binary — see [`managed::Policy::refresh`].
/// * **It never overwrites a hand edit.** Content that matches no text
///   skillrank has ever shipped belongs to the user; it is named and left.
///
/// One honest limitation: a release that predates this function cannot call it,
/// so an install upgrading *into* this feature swaps the binary and stops
/// there. Its next `skillrank update` (or any `skillrank setup`, or re-running
/// `install.sh`, which calls setup) is what actually writes the new text.
fn refresh_managed_files() {
    // The same target list `setup` installs, so a refresh can never drift from
    // what was written in the first place — and, like `setup`, it refuses to
    // invent a home directory. Without one there is no installed Skill to
    // refresh, and certainly no licence to write one into the caller's working
    // directory, which is what this used to do.
    let targets = match crate::setup::managed_targets() {
        Ok(targets) => targets,
        Err(reason) => {
            eprintln!("Not refreshing the installed Skill and command: {reason}");
            return;
        }
    };
    let mut state = managed::load_default_state();
    let triggers = state.resolve_triggers(None);
    let mut enabled_agent_initiative = false;
    for report in managed::refresh(&targets, triggers, &mut state) {
        if let Some(line) = report.message() {
            println!("{line}");
        }
        enabled_agent_initiative |= report.enabled_agent_initiative();
    }
    if enabled_agent_initiative {
        // This refresh just replaced a user-only Skill with the situational
        // one, i.e. it enabled agent-initiated discovery on an install that
        // predates the feature. That is a change in what the agent may do
        // without being asked, so it is stated with its off switch rather than
        // folded into a "Refreshed" line.
        crate::setup::print_trigger_note(managed::Triggers::Situational);
    }
    if let Err(e) = managed::save_default_state(&state) {
        eprintln!("Could not record skillrank's setup state in ~/.skillrank/setup.json: {e}");
    }
}

/// A release GitHub reported. `tag_name` is remote input that ends up in a
/// download path, so it is validated ([`is_safe_tag`]) before a `Release` is
/// handed out and again in [`asset_url`].
pub(crate) struct Release {
    tag_name: String,
}

impl Release {
    /// The release version without the tag's `v` prefix.
    pub(crate) fn version(&self) -> &str {
        self.tag_name.trim_start_matches('v')
    }
}

/// Download this platform's asset for `release`, verify it against the SHA-256
/// published beside it, and swap it over the running binary. `timeout` bounds
/// the transfer. `Err` carries a message that is ready to print as-is, and
/// means nothing was written.
pub(crate) fn apply(release: &Release, timeout: Duration) -> Result<(), String> {
    let asset = asset_name(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        format!(
            "error: unsupported platform: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let url = asset_url(&release.tag_name, asset)?;
    let agent = agent(Some(timeout));
    let bytes = download(&agent, &url, MAX_ASSET_BYTES).map_err(|e| format!("error: {e}"))?;
    if bytes.len() < DOWNLOAD_MIN_BYTES {
        return Err(format!(
            "error: downloaded asset is suspiciously small ({} bytes)",
            bytes.len()
        ));
    }
    verify_asset(&bytes, download_text(&agent, &format!("{url}.sha256")))?;

    let exe = std::env::current_exe()
        .map_err(|e| format!("error: could not resolve current executable: {e}"))?;
    replace_exe(&exe, &bytes).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "cannot replace {}: permission denied. Re-run with sudo, or re-run the installer: curl -fsSL skillrank.dev | sh",
                exe.display()
            )
        } else {
            format!("error: cannot replace {}: {e}", exe.display())
        }
    })
}

/// Look up the latest GitHub release. `timeout` bounds the request; the startup
/// check passes one so a dead network can never hang the CLI, while `skillrank
/// update` (which the user is waiting on deliberately) passes none.
pub(crate) fn latest_release(timeout: Option<Duration>) -> Result<Release, String> {
    let resp = agent(timeout)
        .get(LATEST_RELEASE_URL)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(http_error)?;
    let value = resp
        .into_json::<Value>()
        .map_err(|e| format!("could not parse GitHub release response: {e}"))?;
    let tag_name = value
        .get("tag_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "GitHub release response did not include tag_name".to_string())?
        .to_string();
    // Reject an implausible tag here as well as at the URL, so a hostile or
    // broken response cannot even be cached or printed as a version.
    if !is_safe_tag(&tag_name) {
        return Err(format!(
            "GitHub returned an implausible release tag {tag_name:?}"
        ));
    }
    Ok(Release { tag_name })
}

/// A request agent with *both* timeouts pinned. `.timeout()` on its own is not
/// a ceiling: ureq's default agent sets `timeout_connect` to 30s, and that
/// takes precedence, so a network that black-holes packets rather than
/// refusing them used to hang for 30s+ no matter what the caller asked for.
///
/// One gap remains, documented rather than papered over: DNS resolution runs
/// through the blocking std resolver, which has no cancellation API (ureq's own
/// `stream.rs` carries a `TODO: apply deadline to DNS lookup`). Connect, TLS,
/// and every read after them are bounded; a resolver that never answers is not.
fn agent(timeout: Option<Duration>) -> ureq::Agent {
    let mut builder = ureq::builder().user_agent(USER_AGENT);
    if let Some(timeout) = timeout {
        builder = builder.timeout_connect(timeout).timeout(timeout);
    }
    builder.build()
}

/// Read at most `max_bytes`, and fail rather than truncate. A plain
/// `read_to_end` on a response body is unbounded: whatever is on the other end
/// decides how much memory this process uses.
fn download(agent: &ureq::Agent, url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    let resp = agent.get(url).call().map_err(http_error)?;
    let mut bytes = Vec::new();
    resp.into_reader()
        // One byte past the cap, so hitting it is detectable instead of
        // silently producing a truncated file.
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| format!("could not read {url}: {e}"))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!("{url} is larger than the {max_bytes} byte limit"));
    }
    Ok(bytes)
}

fn download_text(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let bytes = download(agent, url, MAX_CHECKSUM_BYTES)?;
    String::from_utf8(bytes).map_err(|_| format!("{url} is not valid UTF-8"))
}

/// Fail closed, exactly like `install.sh`: an asset whose published checksum
/// does not match — or could not be fetched at all — is not installed.
///
/// `published` is the *result* of fetching `<asset>.sha256`, so the "no
/// checksum" case is a refusal rather than a silent skip. That is the whole
/// point: without it, `apply` trusts whatever bytes the network returned and
/// chmods them over the running binary, which is precisely the guarantee
/// `SECURITY.md` says SkillRank keeps.
fn verify_asset(bytes: &[u8], published: Result<String, String>) -> Result<(), String> {
    let published = published.map_err(|e| {
        format!(
            "error: could not fetch the release checksum: {e}\n\
             Refusing to install an unverified binary."
        )
    })?;
    let expected = parse_sha256(&published).ok_or_else(|| {
        "error: the published checksum is not a SHA-256 digest.\n\
         Refusing to install an unverified binary."
            .to_string()
    })?;
    let actual = sha256_hex(bytes);
    if actual != expected {
        return Err(format!(
            "error: checksum mismatch — refusing to install.\n  expected: {expected}\n  actual:   {actual}"
        ));
    }
    Ok(())
}

/// The published `.sha256` is a bare lowercase digest (see
/// `.github/workflows/release.yml`), but accept the `<digest>  <filename>` form
/// `shasum` prints by default so a hand-published checksum still verifies.
fn parse_sha256(published: &str) -> Option<String> {
    let token = published.split_whitespace().next()?;
    let is_digest = token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit());
    is_digest.then(|| token.to_ascii_lowercase())
}

/// Build the download URL for `tag`, refusing anything that is not a plain
/// release tag and re-checking the formatted result.
///
/// The tag comes from GitHub's JSON, and interpolating an unvalidated one is
/// enough to move the download to a different repository: a tag of
/// `../../../../attacker/evil/releases/download/v9` normalizes to
/// `https://github.com/attacker/evil/…`, served under a genuine github.com
/// certificate. Registry slugs get the same treatment before they are joined
/// onto a path (`skillrank_core::install::is_safe_slug`); this is that
/// discipline applied to the one other piece of remote input that becomes a
/// path.
fn asset_url(tag: &str, asset: &str) -> Result<String, String> {
    if !is_safe_tag(tag) {
        return Err(format!(
            "error: refusing to download an update from an implausible release tag {tag:?}"
        ));
    }
    let url = format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{asset}");
    // Belt and braces: whatever the tag was, the URL actually fetched has to
    // still be this repo's release-download path plus exactly two segments.
    let path = url
        .strip_prefix(RELEASE_DOWNLOAD_PREFIX)
        .unwrap_or_default();
    let mut segments = path.split('/');
    let plain = segments.clone().count() == 2
        && segments.all(|seg| !seg.is_empty() && seg != "." && seg != "..");
    if !plain {
        return Err(format!("error: refusing to download an update from {url}"));
    }
    Ok(url)
}

/// `^v?[0-9A-Za-z][0-9A-Za-z.+_-]*$`, with `..` rejected outright and a length
/// cap. Hand-rolled to keep the CLI dependency-light.
fn is_safe_tag(tag: &str) -> bool {
    if tag.is_empty() || tag.len() > MAX_TAG_LEN || tag.contains("..") {
        return false;
    }
    // `v?` only consumes the prefix when something follows it, mirroring the
    // regex's backtracking: a tag of exactly "v" is still just a first char.
    let body = tag
        .strip_prefix('v')
        .filter(|rest| !rest.is_empty())
        .unwrap_or(tag);
    let mut chars = body.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '_' | '-'))
}

fn replace_exe(exe: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let tmp = temp_path(dir);
    let write_result = write_executable(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, exe));
    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write_result
}

fn write_executable(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn temp_path(dir: &Path) -> PathBuf {
    dir.join(format!(
        ".skillrank-update-{}-{}.tmp",
        std::process::id(),
        env!("CARGO_PKG_VERSION")
    ))
}

fn http_error(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, resp) => {
            let body = summarize_body(&resp.into_string().unwrap_or_default());
            if body.is_empty() {
                format!("HTTP {code}")
            } else {
                format!("HTTP {code}: {body}")
            }
        }
        ureq::Error::Transport(t) => t.to_string(),
    }
}

/// One short line. Bodies here are either a JSON error from the GitHub API or a
/// CDN's HTML error page, and pasting a whole page under an already-actionable
/// message ("Refusing to install an unverified binary") buries it.
fn summarize_body(body: &str) -> String {
    const MAX_CHARS: usize = 200;
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_CHARS {
        return collapsed;
    }
    collapsed.chars().take(MAX_CHARS).chain(['…']).collect()
}

fn asset_name(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("skillrank-macos-aarch64"),
        ("macos", "x86_64") => Some("skillrank-macos-x64"),
        ("linux", "x86_64") => Some("skillrank-linux-x64"),
        ("linux", "aarch64") => Some("skillrank-linux-aarch64"),
        _ => None,
    }
}

/// Numeric, dot-separated comparison. Non-numeric parts (a `-rc.1` suffix, a
/// malformed tag) parse as 0, so a prerelease sorts above the release it
/// precedes — harmless because GitHub's `releases/latest` never returns a
/// prerelease, and deliberately unchanged so `skillrank update` behaves exactly
/// as it did before the startup check started sharing this function.
pub(crate) fn is_newer(latest: &str, current: &str) -> bool {
    let latest_parts = version_parts(latest);
    let current_parts = version_parts(current);
    let len = latest_parts.len().max(current_parts.len());
    for i in 0..len {
        let latest_part = latest_parts.get(i).copied().unwrap_or(0);
        let current_part = current_parts.get(i).copied().unwrap_or(0);
        if latest_part != current_part {
            return latest_part > current_part;
        }
    }
    false
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_name_maps_supported_platforms() {
        assert_eq!(
            asset_name("macos", "aarch64"),
            Some("skillrank-macos-aarch64")
        );
        assert_eq!(asset_name("macos", "x86_64"), Some("skillrank-macos-x64"));
        assert_eq!(asset_name("linux", "x86_64"), Some("skillrank-linux-x64"));
        assert_eq!(
            asset_name("linux", "aarch64"),
            Some("skillrank-linux-aarch64")
        );
    }

    #[test]
    fn asset_name_rejects_unsupported_platform() {
        assert_eq!(asset_name("windows", "x86_64"), None);
    }

    #[test]
    fn is_newer_compares_numeric_parts() {
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.10.0", "0.9.0"));
    }

    #[test]
    fn is_newer_treats_equal_and_padded_versions_as_not_newer() {
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(!is_newer("1.2.0", "1.2"));
        assert!(!is_newer("", ""));
        assert!(is_newer("1.2.1", "1.2"));
    }

    #[test]
    fn is_newer_never_downgrades_on_malformed_input() {
        // Unparseable parts read as 0, so garbage can never look newer than a
        // real version — the startup check stays quiet instead of nagging.
        assert!(!is_newer("not-a-version", "0.1.4"));
        assert!(!is_newer("", "0.1.4"));
        assert!(!is_newer("v0.1.4", "0.1.4"));
        assert!(is_newer("0.1.4", "not-a-version"));
    }

    #[test]
    fn is_newer_sorts_prereleases_above_their_base_version() {
        // Documented, pre-existing behaviour: `-rc` is not understood, so the
        // suffix becomes a trailing 0 and `.1` an extra segment. It costs
        // nothing in practice because GitHub's releases/latest never returns a
        // prerelease, and the startup check reads exactly that endpoint.
        assert!(is_newer("0.2.0-rc.1", "0.1.4"));
        assert!(is_newer("0.2.0-rc.1", "0.2.0"));
        assert!(!is_newer("0.2.0-rc", "0.2.0"));
        assert!(!is_newer("0.1.9-rc.1", "0.2.0"));
    }

    #[test]
    fn verify_asset_accepts_the_published_digest() {
        let bytes = b"skillrank-release-asset";
        let digest = "82ad0a9afa35be95063abf382870b906c94d1fcae64ce66d91ad577a5088a21f";
        assert_eq!(verify_asset(bytes, Ok(format!("{digest}\n"))), Ok(()));
        // The `shasum`-style "<digest>  <filename>" form and upper case both
        // verify, so a hand-published checksum is not a false alarm.
        assert_eq!(
            verify_asset(bytes, Ok(format!("{digest}  skillrank-macos-aarch64\n"))),
            Ok(())
        );
        assert_eq!(verify_asset(bytes, Ok(digest.to_ascii_uppercase())), Ok(()));
    }

    #[test]
    fn verify_asset_rejects_a_checksum_mismatch() {
        // The whole point: bytes that are not the release's bytes never reach
        // `replace_exe`.
        let err = verify_asset(
            b"tampered",
            Ok("82ad0a9afa35be95063abf382870b906c94d1fcae64ce66d91ad577a5088a21f".into()),
        )
        .expect_err("a mismatched digest must be rejected");
        assert!(err.contains("checksum mismatch"), "{err}");
        assert!(err.contains("refusing to install"), "{err}");
    }

    #[test]
    fn verify_asset_fails_closed_without_a_usable_checksum() {
        // Mirrors install.sh: could not fetch it, or what came back is not a
        // digest -> refuse. Never "verified nothing, install anyway".
        let unfetchable = verify_asset(b"payload", Err("HTTP 404".into()))
            .expect_err("a missing checksum must be rejected");
        assert!(unfetchable.contains("Refusing to install an unverified binary"));
        assert!(unfetchable.contains("HTTP 404"), "{unfetchable}");

        for body in [
            "",
            "   \n",
            "not-a-digest",
            "<html>404</html>",
            // 63 and 65 hex characters, and 64 non-hex characters.
            "82ad0a9afa35be95063abf382870b906c94d1fcae64ce66d91ad577a5088a21",
            "82ad0a9afa35be95063abf382870b906c94d1fcae64ce66d91ad577a5088a21ff",
            "zzad0a9afa35be95063abf382870b906c94d1fcae64ce66d91ad577a5088a21f",
        ] {
            let err = verify_asset(b"payload", Ok(body.to_string()))
                .expect_err("a checksum that is not a digest must be rejected");
            assert!(
                err.contains("Refusing to install an unverified binary"),
                "{body:?}: {err}"
            );
        }
    }

    #[test]
    fn http_error_bodies_are_summarized_to_one_short_line() {
        // A GitHub 404 for a missing `.sha256` is served as a full HTML page;
        // it must not bury the "Refusing to install an unverified binary" line
        // underneath it.
        let html =
            "<!DOCTYPE HTML>\n<html>\n  <body>\n    <h1>Error response</h1>\n  </body>\n</html>";
        assert_eq!(
            summarize_body(html),
            "<!DOCTYPE HTML> <html> <body> <h1>Error response</h1> </body> </html>"
        );
        assert_eq!(summarize_body("  \n\t "), "");
        let long = summarize_body(&"x".repeat(500));
        assert_eq!(long.chars().count(), 201);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn asset_url_builds_this_repos_release_path() {
        assert_eq!(
            asset_url("v0.2.0", "skillrank-macos-aarch64").as_deref(),
            Ok("https://github.com/buildbetter-app/skillrank/releases/download/v0.2.0/skillrank-macos-aarch64")
        );
    }

    #[test]
    fn asset_url_rejects_a_traversal_tag() {
        // Verified against the real binary before this check existed: the tag
        // below normalizes to https://github.com/attacker/evil/… and downloads
        // under a genuine github.com certificate.
        for tag in [
            "../../../../attacker/evil/releases/download/v9",
            "..",
            "v0.2.0/../../../../attacker/evil/releases/download/v9",
            "v0.2.0/..",
        ] {
            let err = asset_url(tag, "skillrank-macos-aarch64")
                .expect_err("a traversal tag must never become a download URL");
            assert!(err.contains("refusing to download"), "{tag:?}: {err}");
        }
    }

    #[test]
    fn safe_tags_are_plain_release_tags() {
        for tag in [
            "v0.1.4",
            "0.1.4",
            "v1.2.3-rc.1",
            "v1.2.3+build_7",
            "v",
            "v10",
        ] {
            assert!(is_safe_tag(tag), "expected {tag:?} to be accepted");
        }
        for tag in [
            "",
            " ",
            "..",
            "../evil",
            "v../evil",
            "v0.2.0/asset",
            "/v0.2.0",
            "-v0.2.0",
            ".0.2.0",
            "v0.2.0 ",
            "v0.2.0?x=1",
            "v0.2.0#frag",
            "v0.2.0%2f..",
            "https://attacker.example/v1",
            "v0.2.0\\..",
            &"v".repeat(MAX_TAG_LEN + 1),
        ] {
            assert!(!is_safe_tag(tag), "expected {tag:?} to be refused");
        }
    }

    #[test]
    fn release_version_strips_the_tag_prefix() {
        assert_eq!(
            Release {
                tag_name: "v0.2.0".into()
            }
            .version(),
            "0.2.0"
        );
        assert_eq!(
            Release {
                tag_name: "0.2.0".into()
            }
            .version(),
            "0.2.0"
        );
    }
}
