//! The files skillrank owns on the user's machine — the agent Skill and the
//! `/skillrank` slash command — plus the small amount of state that has to
//! outlive a single command.
//!
//! Everything here exists to make three promises keepable:
//!
//! * **A hand edit is never destroyed.** Before replacing a file, compare what
//!   is on disk against the set of texts skillrank has ever shipped. A match is
//!   our own output and is safe to replace; anything else belongs to the user
//!   and is left exactly as it is. This closes a real defect: `setup` used to
//!   overwrite `SKILL.md` unconditionally while backing up the MCP config files
//!   it touched, so the one file people actually tune was the one file with no
//!   safety net.
//! * **An overwrite is recoverable.** The previous bytes go to
//!   `<path>.skillrank-bak` first, the same convention `setup` already uses for
//!   `~/.claude.json` and `~/.codex/config.toml`. The slot is not a ring
//!   buffer: a backup holding text skillrank never shipped is the only copy of
//!   somebody's hand edit, and a later routine refresh must not spend it.
//!   Backups also inherit the source file's permissions, because the same
//!   convention is applied to config files that are 0600 for a reason.
//! * **Absence is a choice.** If skillrank installed a file and the user later
//!   deleted it, neither `setup` nor `update` quietly puts it back. `--force`
//!   is the way to say "no, really".
//!
//! The trigger preference lives here too, because the whole point of an off
//! switch is that it survives the next `setup` and the next `update`.

use skillrank_core::config;
use skillrank_core::{compute_content_hash, hashes_equal};
use std::path::{Path, PathBuf};

/// The Skill text this release ships: situational triggers, so an agent can
/// recognise its own stuck-ness without the user naming skillrank.
pub const SKILL_MD: &str = include_str!("skillrank_skill.md");

/// The pre-rewrite Skill text, kept byte-identical to what 0.1.1–0.1.4 shipped.
///
/// It has two jobs at once, and both require it to stay frozen: it is what
/// `setup --triggers=user-only` writes, and its hash is how we recognise an
/// untouched 0.1.1–0.1.4 install when deciding whether a refresh is safe. Do
/// not "improve" it — improve [`SKILL_MD`] instead.
pub const SKILL_MD_USER_ONLY: &str = include_str!("skillrank_skill_user_only.md");

pub const COMMAND_MD: &str = include_str!("skillrank_command.md");

/// Canonical hashes of Skill texts skillrank shipped in earlier releases and no
/// longer ships. Both current variants are recognised automatically, so this
/// only needs the ones whose source is gone.
///
/// When you change `skillrank_skill.md`, append the outgoing text's hash here —
/// otherwise every existing install looks hand-edited to [`write_managed`] and
/// silently stops receiving updates.
const RETIRED_SKILL_HASHES: &[&str] = &[
    // v0.1.0, superseded by a7a55bd (it still told people to `go install`).
    "sha256:d3a1e2bed955ea84e16361e65b2ff0d721aeddb3b65ae376e6ab8138e3d29405",
];

/// Same idea for the slash command. Empty because the text has not changed
/// since it was introduced in v0.1.1.
const RETIRED_COMMAND_HASHES: &[&str] = &[];

/// Where the preference and install bookkeeping live. `~/.skillrank` already
/// holds the auth token and the update-check cache.
const STATE_FILE: &str = "setup.json";

/// Which Skill description gets written.
///
/// The rewrite is the default because a description that only fires when the
/// user already knows about skillrank cannot reach a user who does not — but
/// the choice has to be reversible in one command, and permanently.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Triggers {
    /// Fires on the agent's own situation as well as on user requests.
    #[default]
    Situational,
    /// Pre-rewrite behaviour: only fires when the user asks about skills.
    UserOnly,
}

impl Triggers {
    /// Accepts the value of `--triggers`. `default` and `situational` are the
    /// same thing; both spellings exist so turning the switch back on is
    /// guessable.
    pub fn parse(value: &str) -> Option<Triggers> {
        match value.trim().to_ascii_lowercase().as_str() {
            "user-only" | "user_only" | "useronly" => Some(Triggers::UserOnly),
            "default" | "situational" | "agent" => Some(Triggers::Situational),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Triggers::Situational => "situational",
            Triggers::UserOnly => "user-only",
        }
    }

    /// One line explaining what the variant does, for `setup --print`.
    pub fn describe(self) -> &'static str {
        match self {
            Triggers::Situational => {
                "situational — also fires when the agent notices its own situation (unfamiliar tool, repeated failure)"
            }
            Triggers::UserOnly => {
                "user-only — fires only when the user asks about skills (agent-initiated trigger off)"
            }
        }
    }

    /// The Skill text for this variant.
    pub fn skill_text(self) -> &'static str {
        match self {
            Triggers::Situational => SKILL_MD,
            Triggers::UserOnly => SKILL_MD_USER_ONLY,
        }
    }
}

/// Which managed file a path holds. Each kind knows every text skillrank has
/// ever written there, which is what makes user-edit detection possible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Skill,
    Command,
}

impl Kind {
    /// The text this release wants at that path.
    pub fn desired(self, triggers: Triggers) -> &'static str {
        match self {
            Kind::Skill => triggers.skill_text(),
            Kind::Command => COMMAND_MD,
        }
    }

    /// Canonical hashes of everything skillrank has ever written for this kind:
    /// both current Skill variants (so flipping the trigger switch back and
    /// forth never looks like tampering) plus the retired releases.
    pub fn shipped_hashes(self) -> Vec<String> {
        match self {
            Kind::Skill => {
                let mut hashes = vec![
                    compute_content_hash(SKILL_MD),
                    compute_content_hash(SKILL_MD_USER_ONLY),
                ];
                hashes.extend(RETIRED_SKILL_HASHES.iter().map(|h| h.to_string()));
                hashes
            }
            Kind::Command => {
                let mut hashes = vec![compute_content_hash(COMMAND_MD)];
                hashes.extend(RETIRED_COMMAND_HASHES.iter().map(|h| h.to_string()));
                hashes
            }
        }
    }
}

/// One managed file on this machine.
pub struct Target {
    /// Human name for messages, e.g. "skillrank Skill for Claude Code".
    pub label: String,
    pub path: PathBuf,
    pub kind: Kind,
}

/// How aggressive a write is allowed to be.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    /// Write the file when it does not exist. `setup` installs; `update` only
    /// refreshes what is already there, so a machine that opted out of setup
    /// never grows a Skill file behind the user's back.
    pub create: bool,
    /// Overwrite a hand-edited file, and re-create one the user deleted. Always
    /// an explicit `--force`, never implied.
    pub force: bool,
}

impl Policy {
    pub fn install(force: bool) -> Policy {
        Policy {
            create: true,
            force,
        }
    }

    pub fn refresh() -> Policy {
        Policy {
            create: false,
            force: false,
        }
    }
}

/// What a write did, plus the one transition worth announcing on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Written {
    pub outcome: Outcome,
    /// The file held the frozen user-only Skill text and now holds the
    /// situational one — this write is what turned agent-initiated discovery on
    /// for an install that never asked for it. Callers say so out loud; a
    /// change to what the agent may do unprompted must not arrive as a
    /// one-word "Refreshed".
    pub enabled_agent_initiative: bool,
}

impl Written {
    fn plain(outcome: Outcome) -> Written {
        Written {
            outcome,
            enabled_agent_initiative: false,
        }
    }
}

/// What a write actually did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Created,
    Updated,
    /// Already byte-for-byte what we wanted: no write, no backup.
    Unchanged,
    /// On-disk content matches no text we have ever shipped. Left alone.
    UserEdited,
    /// We installed it once, the user deleted it. Not re-created.
    UserRemoved,
    /// Not present and we have no record of installing it, with `create` off.
    Absent,
}

/// Write `kind`'s text to `path`, honouring every promise in the module docs.
/// `state` is read to spot a deliberate deletion and updated to record what is
/// installed; the caller persists it once at the end.
pub fn write_managed(
    path: &Path,
    kind: Kind,
    triggers: Triggers,
    policy: Policy,
    state: &mut State,
) -> std::io::Result<Written> {
    let desired = kind.desired(triggers);
    match read_existing(path)? {
        Some(existing) => {
            // Bytes rather than a string: a file that is not valid UTF-8 cannot
            // be anything we wrote, so it reads as a user edit instead of
            // failing the whole command over one odd file.
            let text = std::str::from_utf8(&existing).ok();
            let hash = text.map(compute_content_hash);
            if hash
                .as_deref()
                .is_some_and(|h| hashes_equal(h, &compute_content_hash(desired)))
            {
                state.record_installed(path);
                return Ok(Written::plain(Outcome::Unchanged));
            }
            let ours = hash
                .as_deref()
                .is_some_and(|h| is_shipped(h, &kind.shipped_hashes()));
            if !policy.force && !ours {
                return Ok(Written::plain(Outcome::UserEdited));
            }
            let enabled_agent_initiative = kind == Kind::Skill
                && triggers == Triggers::Situational
                && hash
                    .as_deref()
                    .is_some_and(|h| hashes_equal(h, &compute_content_hash(SKILL_MD_USER_ONLY)));
            backup_managed(path, kind, &existing)?;
            write_file(path, desired)?;
            state.record_installed(path);
            Ok(Written {
                outcome: Outcome::Updated,
                enabled_agent_initiative,
            })
        }
        None => {
            if state.was_installed(path) && !policy.force {
                return Ok(Written::plain(Outcome::UserRemoved));
            }
            if !policy.create && !policy.force {
                return Ok(Written::plain(Outcome::Absent));
            }
            write_file(path, desired)?;
            state.record_installed(path);
            Ok(Written::plain(Outcome::Created))
        }
    }
}

/// Bring every target up to the text this release ships, without ever failing
/// the caller — `skillrank update` calls it once the update itself has already
/// succeeded, so a refresh problem must be reported, not escalated. Errors are
/// carried per target in [`Report`] rather than short-circuiting, so one
/// unwritable path cannot hide the others.
pub fn refresh(targets: &[Target], triggers: Triggers, state: &mut State) -> Vec<Report> {
    targets
        .iter()
        .map(|target| Report {
            label: target.label.clone(),
            path: target.path.clone(),
            result: write_managed(
                &target.path,
                target.kind,
                triggers,
                Policy::refresh(),
                state,
            ),
        })
        .collect()
}

/// The outcome of one target in a [`refresh`], including the failure case.
pub struct Report {
    pub label: String,
    pub path: PathBuf,
    pub result: std::io::Result<Written>,
}

impl Report {
    /// One line worth printing, or `None` when there is nothing to say. Silence
    /// is the right answer for the common cases (already current, never
    /// installed) — an update should not narrate its own bookkeeping.
    pub fn message(&self) -> Option<String> {
        let outcome = match &self.result {
            Ok(written) => written.outcome,
            Err(e) => {
                return Some(format!(
                    "Could not refresh {} ({}): {e}",
                    self.label,
                    self.show()
                ))
            }
        };
        match outcome {
            Outcome::Created => Some(format!("Installed {} ({})", self.label, self.show())),
            Outcome::Updated => Some(format!("Refreshed {} ({})", self.label, self.show())),
            Outcome::Unchanged | Outcome::Absent | Outcome::UserRemoved => None,
            Outcome::UserEdited => Some(format!(
                "Left your edited {} alone ({}). A newer version ships with this release; `skillrank setup --force` replaces it (the old copy is kept as {}.skillrank-bak).",
                self.label,
                self.show(),
                self.show()
            )),
        }
    }

    /// Whether this write is the one that turned agent-initiated discovery on.
    /// The caller owes the user a sentence when it is.
    pub fn enabled_agent_initiative(&self) -> bool {
        matches!(&self.result, Ok(written) if written.enabled_agent_initiative)
    }

    fn show(&self) -> String {
        self.path.display().to_string()
    }
}

/// `None` when the file does not exist; a read error for any other reason is
/// surfaced, because silently treating an unreadable file as absent is how you
/// end up overwriting it.
fn read_existing(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn is_shipped(hash: &str, shipped: &[String]) -> bool {
    shipped.iter().any(|known| hashes_equal(hash, known))
}

/// Whether these bytes are a text skillrank itself has shipped for `kind`, and
/// therefore reproducible from any release rather than somebody's only copy.
fn is_ours(bytes: &[u8], kind: Kind) -> bool {
    std::str::from_utf8(bytes)
        .ok()
        .map(compute_content_hash)
        .is_some_and(|hash| is_shipped(&hash, &kind.shipped_hashes()))
}

fn write_file(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

fn backup_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.skillrank-bak", path.display()))
}

/// Copy the current bytes to `<path>.skillrank-bak` before replacing them, at
/// the source file's own permissions.
///
/// Used for the MCP config files, whose every backup is the user's own content
/// and whose newest copy is the one worth keeping.
pub fn backup(path: &Path, data: &[u8]) -> std::io::Result<()> {
    copy_permissions_of(path, &backup_path(path), data)
}

/// Back up a managed file, without ever spending the slot on something
/// worthless.
///
/// A single fixed backup path is fine while everything that lands in it is
/// reproducible, and stops being fine the moment it holds a hand edit rescued
/// by `setup --force`: that copy exists nowhere else, and the next ordinary
/// shipped-to-shipped refresh used to overwrite it, so "an overwrite is
/// recoverable" quietly expired one update after it was needed. Text skillrank
/// never wrote is therefore never displaced — a second, genuinely distinct hand
/// edit parks alongside it instead.
fn backup_managed(path: &Path, kind: Kind, data: &[u8]) -> std::io::Result<()> {
    let slot = backup_path(path);
    let Some(parked) = read_existing(&slot)? else {
        return copy_permissions_of(path, &slot, data);
    };
    if is_ours(&parked, kind) {
        // Whatever is in the slot, any release can reprint. Take it.
        return copy_permissions_of(path, &slot, data);
    }
    if parked == data || is_ours(data, kind) {
        // Nothing new to preserve: the same bytes, or bytes that are ours and
        // so cost nothing to lose. Leave the user's copy exactly where it is.
        return Ok(());
    }
    copy_permissions_of(path, &next_free_backup_path(&slot), data)
}

/// `<slot>.1`, `<slot>.2`, … — only reached by repeated `--force` over
/// repeatedly hand-edited files, so in practice this stops at 1. The cap keeps
/// a pathological directory from spinning; at that point displacing a numbered
/// backup is the least-bad option left, and the primary slot is still intact.
fn next_free_backup_path(slot: &Path) -> PathBuf {
    let mut candidate = PathBuf::new();
    for n in 1..=99 {
        candidate = PathBuf::from(format!("{}.{n}", slot.display()));
        if !candidate.exists() {
            break;
        }
    }
    candidate
}

/// Write `data` to `dest` with the permissions `source` carries.
///
/// `std::fs::write` creates 0644. Two of the files this is used on —
/// `~/.claude.json` and `~/.codex/config.toml` — are routinely 0600 because
/// they hold *other tools'* OAuth tokens and API keys, so a 0644 copy is those
/// secrets published to every other account on the machine, in a file nothing
/// ever cleans up. When the source's mode cannot be read, fall back to 0600:
/// too strict is a permission error the user can see and fix, too loose is a
/// leak they never find out about.
fn copy_permissions_of(source: &Path, dest: &Path, data: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mode = std::fs::metadata(source)
            .map(|meta| meta.permissions().mode() & 0o777)
            .unwrap_or(0o600);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(dest)?;
        file.write_all(data)?;
        // `.mode()` applies only to a file this call creates, and the umask
        // narrows it; setting it afterwards covers the reused-backup case and
        // makes the result exactly the source's mode either way.
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(mode))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = source;
        std::fs::write(dest, data)
    }
}

/// Preferences and install bookkeeping, stored as JSON in `~/.skillrank`.
///
/// Deliberately tolerant on read: a state file we cannot understand is treated
/// as no state file, exactly like the update-check cache. Losing the record
/// costs a re-created file at worst; refusing to run because of it would be
/// much worse.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// `None` means *no preference on record* — a fresh machine, a deleted
    /// state file, or one that would not parse. Kept distinct from
    /// `Some(Triggers::Situational)` on purpose: a default is not a choice, and
    /// code that cannot tell them apart ends up reporting an assumption as the
    /// user's decision.
    pub triggers: Option<Triggers>,
    /// Absolute paths of managed files skillrank has installed at least once.
    /// Recorded here plus missing on disk means the user removed it on purpose.
    pub installed: Vec<String>,
}

impl State {
    pub fn was_installed(&self, path: &Path) -> bool {
        let path = path.display().to_string();
        self.installed.iter().any(|known| known == &path)
    }

    fn record_installed(&mut self, path: &Path) {
        let path = path.display().to_string();
        if !self.installed.iter().any(|known| known == &path) {
            self.installed.push(path);
        }
    }

    /// The variant to write: an explicit `--triggers` wins, then the recorded
    /// preference, then the default. No preference on record never resolves to
    /// a stored choice — that is the whole difference between honouring a
    /// decision and inventing one.
    pub fn resolve_triggers(&self, requested: Option<Triggers>) -> Triggers {
        requested.or(self.triggers).unwrap_or_default()
    }
}

pub fn state_path() -> std::io::Result<PathBuf> {
    Ok(config::home()?.join(STATE_FILE))
}

pub fn load_state(path: &Path) -> State {
    parse_state(&std::fs::read_to_string(path).unwrap_or_default()).unwrap_or_default()
}

/// Persist the state, atomically.
///
/// A plain `fs::write` that dies mid-flight (full disk, killed process) leaves
/// a truncated `setup.json`, which parses as no preference at all — so the next
/// `update` restores the situational trigger somebody had explicitly turned
/// off. Writing a sibling and renaming it means the file at `path` is only ever
/// a complete one.
pub fn save_state(path: &Path, state: &State) -> std::io::Result<()> {
    let mut body = serde_json::Map::new();
    if let Some(triggers) = state.triggers {
        body.insert("triggers".to_string(), triggers.as_str().into());
    }
    body.insert("installed".to_string(), state.installed.clone().into());
    let body = serde_json::Value::Object(body);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let written =
        std::fs::write(&tmp, format!("{body}\n")).and_then(|_| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Load state from the default location; a machine with no readable home just
/// gets defaults, which behave exactly like a fresh install.
pub fn load_default_state() -> State {
    state_path()
        .map(|path| load_state(&path))
        .unwrap_or_default()
}

/// Persist to the default location.
///
/// The error is returned rather than dropped. A `--triggers=user-only` that
/// could not be written down is an off switch that silently turns itself back
/// on at the next `update`, and the one moment anyone can act on that is while
/// the command that failed to record it is still on screen.
pub fn save_default_state(state: &State) -> std::io::Result<()> {
    save_state(&state_path()?, state)
}

fn parse_state(body: &str) -> Option<State> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    // An unrecognised spelling reads as *unknown*, not as a preference we made
    // up on the user's behalf.
    let triggers = value
        .get("triggers")
        .and_then(|v| v.as_str())
        .and_then(Triggers::parse);
    let installed = value
        .get("installed")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some(State {
        triggers,
        installed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "skillrank-managed-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn bak(path: &Path) -> PathBuf {
        PathBuf::from(format!("{}.skillrank-bak", path.display()))
    }

    /// [`write_managed`] reduced to the outcome, which is all most tests care
    /// about; the ones about the trigger transition read `Written` directly.
    fn write_skill(path: &Path, triggers: Triggers, policy: Policy, state: &mut State) -> Outcome {
        write_managed(path, Kind::Skill, triggers, policy, state)
            .unwrap()
            .outcome
    }

    #[test]
    fn shipped_skill_description_is_situational_and_gated() {
        let description = frontmatter_value(SKILL_MD, "description").expect("description");
        // The whole hypothesis: a state the agent can observe about itself,
        // plus a temporal gate, plus explicit negative gates.
        for clause in [
            "before starting work with a framework, library, or tool this agent has no established approach for",
            "after the second failed attempt at the same problem with no new information",
            "before continuing to improvise",
            "Do NOT use for a single failed command",
            "the next step is already clear",
            "more than once per session",
        ] {
            assert!(
                description.contains(clause),
                "description lost the {clause:?} clause"
            );
        }
        // The user-initiated triggers are correct and cost nothing; they were
        // insufficient alone, not wrong.
        assert!(description.contains("Also use when the user asks to find, choose, install, evaluate, or compare agent skills"));
    }

    #[test]
    fn user_only_variant_keeps_the_pre_rewrite_description() {
        let description =
            frontmatter_value(SKILL_MD_USER_ONLY, "description").expect("description");
        assert!(description.starts_with("Use when the user wants to find, choose, install"));
        assert!(
            !description.contains("second failed attempt"),
            "the off switch must not carry the agent-initiated trigger"
        );
        assert_eq!(
            frontmatter_value(SKILL_MD_USER_ONLY, "name").as_deref(),
            Some("skillrank")
        );
    }

    #[test]
    fn both_variants_have_parseable_frontmatter() {
        for text in [SKILL_MD, SKILL_MD_USER_ONLY] {
            assert!(text.starts_with("---\n"), "missing frontmatter fence");
            let description = frontmatter_value(text, "description").expect("description");
            // Claude Code matches on this string; a stray newline splits it.
            assert!(!description.contains('\n'));
            assert!(description.len() > 200, "suspiciously short description");
        }
    }

    #[test]
    fn body_leads_with_the_agent_initiated_path_and_forbids_installing_on_it() {
        let initiative = SKILL_MD
            .find("## If you got here on your own initiative")
            .expect("agent-initiated section");
        let reference = SKILL_MD
            .find("## When to use which command")
            .expect("command reference");
        assert!(
            initiative < reference,
            "the cheap agent-initiated path must come before the full reference"
        );
        assert!(SKILL_MD.contains("until\n  the user has explicitly said yes in this conversation"));
        assert!(SKILL_MD.contains("One suggestion per session"));
        // The off switch has to be findable at the moment it is wanted.
        assert!(SKILL_MD.contains("skillrank setup --triggers=user-only"));
    }

    #[test]
    fn retired_hashes_cover_the_releases_whose_text_is_gone() {
        // v0.1.0's SKILL.md, recorded so an untouched 0.1.0 install is still
        // recognised as ours and gets refreshed rather than treated as edited.
        assert!(RETIRED_SKILL_HASHES
            .contains(&"sha256:d3a1e2bed955ea84e16361e65b2ff0d721aeddb3b65ae376e6ab8138e3d29405"));
        // v0.1.1–v0.1.4 shipped exactly the frozen user-only text, so they are
        // covered without a literal.
        assert!(Kind::Skill
            .shipped_hashes()
            .contains(&compute_content_hash(SKILL_MD_USER_ONLY)));
        assert!(Kind::Skill
            .shipped_hashes()
            .contains(&compute_content_hash(SKILL_MD)));
    }

    #[test]
    fn install_is_idempotent_and_does_not_back_up_a_no_op() {
        let dir = tmp("idempotent");
        let path = dir.join("SKILL.md");
        let mut state = State::default();

        let first = write_skill(
            &path,
            Triggers::Situational,
            Policy::install(false),
            &mut state,
        );
        let second = write_skill(
            &path,
            Triggers::Situational,
            Policy::install(false),
            &mut state,
        );

        assert_eq!(first, Outcome::Created);
        assert_eq!(second, Outcome::Unchanged);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL_MD);
        assert!(!bak(&path).exists(), "a no-op must not write a backup");
        assert_eq!(state.installed, vec![path.display().to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upgrading_a_shipped_version_backs_up_the_old_text() {
        let dir = tmp("upgrade");
        let path = dir.join("SKILL.md");
        std::fs::write(&path, SKILL_MD_USER_ONLY).unwrap();
        let mut state = State::default();

        let outcome = write_skill(
            &path,
            Triggers::Situational,
            Policy::install(false),
            &mut state,
        );

        assert_eq!(outcome, Outcome::Updated);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL_MD);
        assert_eq!(
            std::fs::read_to_string(bak(&path)).unwrap(),
            SKILL_MD_USER_ONLY,
            "the replaced text must be recoverable"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_hand_edited_file_survives_setup_and_update() {
        let dir = tmp("hand-edited");
        let path = dir.join("SKILL.md");
        let edited = format!("{SKILL_MD_USER_ONLY}\n\n- always use --json\n");
        std::fs::write(&path, &edited).unwrap();
        let mut state = State::default();

        for policy in [Policy::install(false), Policy::refresh()] {
            let outcome = write_skill(&path, Triggers::Situational, policy, &mut state);
            assert_eq!(outcome, Outcome::UserEdited);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
            assert!(!bak(&path).exists(), "nothing was replaced, so no backup");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_that_is_not_even_text_counts_as_the_users() {
        let dir = tmp("not-utf8");
        let path = dir.join("SKILL.md");
        std::fs::write(&path, [0xff, 0xfe, 0x00, 0x01]).unwrap();
        let mut state = State::default();

        let outcome = write_skill(
            &path,
            Triggers::Situational,
            Policy::install(false),
            &mut state,
        );

        // Nothing skillrank ships is invalid UTF-8, so this cannot be ours —
        // and one strange file must not fail the whole of `setup`.
        assert_eq!(outcome, Outcome::UserEdited);
        assert_eq!(std::fs::read(&path).unwrap(), vec![0xff, 0xfe, 0x00, 0x01]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn force_replaces_a_hand_edited_file_but_keeps_a_copy() {
        let dir = tmp("force");
        let path = dir.join("SKILL.md");
        std::fs::write(&path, "totally my own thing\n").unwrap();
        let mut state = State::default();

        let outcome = write_skill(
            &path,
            Triggers::Situational,
            Policy::install(true),
            &mut state,
        );

        assert_eq!(outcome, Outcome::Updated);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL_MD);
        assert_eq!(
            std::fs::read_to_string(bak(&path)).unwrap(),
            "totally my own thing\n"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_deleted_file_is_not_silently_re_created() {
        let dir = tmp("deleted");
        let path = dir.join("SKILL.md");
        let mut state = State::default();
        write_skill(
            &path,
            Triggers::Situational,
            Policy::install(false),
            &mut state,
        );
        std::fs::remove_file(&path).unwrap();

        for policy in [Policy::install(false), Policy::refresh()] {
            let outcome = write_skill(&path, Triggers::Situational, policy, &mut state);
            assert_eq!(outcome, Outcome::UserRemoved);
            assert!(!path.exists(), "absence is a choice");
        }

        let forced = write_skill(
            &path,
            Triggers::Situational,
            Policy::install(true),
            &mut state,
        );
        assert_eq!(forced, Outcome::Created);
        assert!(path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn update_refreshes_what_exists_and_never_installs_what_does_not() {
        let dir = tmp("refresh");
        let stale = dir.join("stale").join("SKILL.md");
        let never = dir.join("never").join("SKILL.md");
        std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
        std::fs::write(&stale, SKILL_MD_USER_ONLY).unwrap();
        let mut state = State::default();

        let targets = vec![
            Target {
                label: "skillrank Skill for Claude Code".into(),
                path: stale.clone(),
                kind: Kind::Skill,
            },
            Target {
                label: "skillrank Skill for Codex".into(),
                path: never.clone(),
                kind: Kind::Skill,
            },
        ];
        let reports = refresh(&targets, Triggers::Situational, &mut state);

        assert_eq!(
            reports[0].result.as_ref().unwrap().outcome,
            Outcome::Updated
        );
        assert_eq!(std::fs::read_to_string(&stale).unwrap(), SKILL_MD);
        assert!(reports[0].message().unwrap().contains("Refreshed"));
        assert_eq!(reports[1].result.as_ref().unwrap().outcome, Outcome::Absent);
        assert!(
            !never.exists(),
            "update must not install where setup did not"
        );
        assert!(reports[1].message().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn update_honours_a_recorded_user_only_preference() {
        let dir = tmp("refresh-user-only");
        let path = dir.join("SKILL.md");
        // A 0.1.0 install, i.e. a retired text: still ours, still refreshable.
        std::fs::write(
            &path,
            "---\nname: skillrank\ndescription: old\n---\nold body\n",
        )
        .unwrap();
        let mut state = State {
            triggers: Some(Triggers::UserOnly),
            installed: vec![path.display().to_string()],
        };

        let triggers = state.resolve_triggers(None);
        let outcome = write_skill(&path, triggers, Policy::install(true), &mut state);

        assert_eq!(outcome, Outcome::Updated);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL_MD_USER_ONLY);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn triggers_parse_covers_the_documented_spellings() {
        assert_eq!(Triggers::parse("user-only"), Some(Triggers::UserOnly));
        assert_eq!(Triggers::parse(" User-Only "), Some(Triggers::UserOnly));
        assert_eq!(Triggers::parse("default"), Some(Triggers::Situational));
        assert_eq!(Triggers::parse("situational"), Some(Triggers::Situational));
        assert_eq!(Triggers::parse("off"), None);
        assert_eq!(Triggers::parse(""), None);
        assert_eq!(Triggers::default(), Triggers::Situational);
    }

    #[test]
    fn an_explicit_flag_beats_the_recorded_preference() {
        let state = State {
            triggers: Some(Triggers::UserOnly),
            ..State::default()
        };
        assert_eq!(state.resolve_triggers(None), Triggers::UserOnly);
        assert_eq!(
            state.resolve_triggers(Some(Triggers::Situational)),
            Triggers::Situational
        );
    }

    #[test]
    fn state_round_trips_and_survives_garbage() {
        let dir = tmp("state");
        let path = dir.join("setup.json");
        let state = State {
            triggers: Some(Triggers::UserOnly),
            installed: vec!["/home/x/.claude/skills/skillrank/SKILL.md".into()],
        };
        save_state(&path, &state).unwrap();
        assert_eq!(load_state(&path), state);

        for body in ["", "   ", "not json", "[]", "null", "{}"] {
            std::fs::write(&path, body).unwrap();
            assert_eq!(
                load_state(&path),
                State::default(),
                "unusable state {body:?} must read as no state"
            );
        }
        // A preference we do not recognise reads as *no* preference, not as one
        // we picked on the user's behalf.
        std::fs::write(&path, r#"{"triggers":"whatever","installed":[1,"/a",""]}"#).unwrap();
        assert_eq!(
            load_state(&path),
            State {
                triggers: None,
                installed: vec!["/a".to_string()],
            }
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_state_file_reads_as_defaults() {
        let missing = std::env::temp_dir().join("skillrank-managed-no-such-state.json");
        let _ = std::fs::remove_file(&missing);
        assert_eq!(load_state(&missing), State::default());
    }

    /// A state file that is gone, truncated, or unreadable must read as "no
    /// preference on record", never as an affirmative Situational — those are
    /// different facts, and only one of them is something the user said.
    #[test]
    fn a_lost_state_file_reads_as_unknown_not_as_situational() {
        let dir = tmp("state-unknown");
        let path = dir.join("setup.json");
        save_state(
            &path,
            &State {
                triggers: Some(Triggers::UserOnly),
                installed: vec![],
            },
        )
        .unwrap();
        assert_eq!(load_state(&path).triggers, Some(Triggers::UserOnly));

        for lost in ["", "{", r#"{"installed":[]}"#] {
            std::fs::write(&path, lost).unwrap();
            assert_eq!(
                load_state(&path).triggers,
                None,
                "a state file that reads as {lost:?} records no preference"
            );
        }
        std::fs::remove_file(&path).unwrap();
        assert_eq!(load_state(&path).triggers, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The off switch is only worth having if it survives everything routine.
    /// `save_state` is atomic so a half-written file cannot exist, and a
    /// recorded user-only is what a later `update` reads back.
    #[test]
    fn the_off_switch_survives_a_save_and_a_later_update() {
        let dir = tmp("off-switch");
        let state_file = dir.join("setup.json");
        let skill = dir.join("SKILL.md");
        std::fs::write(&skill, SKILL_MD_USER_ONLY).unwrap();

        let mut chosen = State {
            triggers: Some(Triggers::UserOnly),
            installed: vec![skill.display().to_string()],
        };
        save_state(&state_file, &chosen).unwrap();
        // No leftover temp file: the write landed as one complete rename.
        let strays: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains("tmp"))
            .collect();
        assert!(strays.is_empty(), "atomic save left {strays:?} behind");

        let reloaded = load_state(&state_file);
        assert_eq!(reloaded.triggers, Some(Triggers::UserOnly));

        // What `update` does: resolve with no flag, then refresh.
        let triggers = reloaded.resolve_triggers(None);
        assert_eq!(triggers, Triggers::UserOnly);
        let targets = vec![Target {
            label: "skillrank Skill for Claude Code".into(),
            path: skill.clone(),
            kind: Kind::Skill,
        }];
        let reports = refresh(&targets, triggers, &mut chosen);
        assert_eq!(
            reports[0].result.as_ref().unwrap().outcome,
            Outcome::Unchanged,
            "an update must not rewrite a Skill that already matches the recorded preference"
        );
        assert_eq!(
            std::fs::read_to_string(&skill).unwrap(),
            SKILL_MD_USER_ONLY,
            "the agent-initiated trigger came back after an update"
        );
        assert!(!reports[0].enabled_agent_initiative());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Losing the state file costs the recorded preference — there is nothing
    /// left to read. What it must not cost is the user's knowledge that the
    /// trigger came back: the write that flips a user-only Skill to the
    /// situational one reports itself, so `update` says so with the off switch.
    #[test]
    fn losing_the_state_file_cannot_revert_the_trigger_silently() {
        let dir = tmp("state-lost");
        let state_file = dir.join("setup.json");
        let skill = dir.join("SKILL.md");
        std::fs::write(&skill, SKILL_MD_USER_ONLY).unwrap();
        save_state(
            &state_file,
            &State {
                triggers: Some(Triggers::UserOnly),
                installed: vec![skill.display().to_string()],
            },
        )
        .unwrap();

        // Exactly the reported reproduction: the state file goes away.
        std::fs::remove_file(&state_file).unwrap();

        let mut state = load_state(&state_file);
        assert_eq!(
            state.triggers, None,
            "a deleted state file must not read as an affirmative preference"
        );
        state.installed = vec![skill.display().to_string()];

        let targets = vec![Target {
            label: "skillrank Skill for Claude Code".into(),
            path: skill.clone(),
            kind: Kind::Skill,
        }];
        let reports = refresh(&targets, state.resolve_triggers(None), &mut state);
        assert_eq!(
            reports[0].result.as_ref().unwrap().outcome,
            Outcome::Updated
        );
        assert!(
            reports[0].enabled_agent_initiative(),
            "the trigger came back with nothing telling the user, which is the defect"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Turning the situational trigger on for an install that predates it is a
    /// change to what the agent may do unprompted, so the write reports it and
    /// `update` can say so. Every other write must stay quiet.
    #[test]
    fn enabling_the_agent_initiated_trigger_is_reported() {
        let dir = tmp("announce");
        let upgraded = dir.join("upgraded").join("SKILL.md");
        let already = dir.join("already").join("SKILL.md");
        let command = dir.join("command").join("skillrank.md");
        for (path, body) in [
            (&upgraded, SKILL_MD_USER_ONLY),
            (&already, SKILL_MD),
            (&command, SKILL_MD_USER_ONLY),
        ] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        let mut state = State::default();

        let flipped = write_managed(
            &upgraded,
            Kind::Skill,
            Triggers::Situational,
            Policy::refresh(),
            &mut state,
        )
        .unwrap();
        assert_eq!(flipped.outcome, Outcome::Updated);
        assert!(
            flipped.enabled_agent_initiative,
            "a user-only Skill becoming situational must be announced"
        );

        // Already situational: nothing changed, nothing to announce.
        let noop = write_managed(
            &already,
            Kind::Skill,
            Triggers::Situational,
            Policy::refresh(),
            &mut state,
        )
        .unwrap();
        assert_eq!(noop.outcome, Outcome::Unchanged);
        assert!(!noop.enabled_agent_initiative);

        // Going the other way is the off switch, which `setup` already narrates.
        std::fs::write(&already, SKILL_MD).unwrap();
        let back = write_managed(
            &already,
            Kind::Skill,
            Triggers::UserOnly,
            Policy::install(false),
            &mut state,
        )
        .unwrap();
        assert_eq!(back.outcome, Outcome::Updated);
        assert!(!back.enabled_agent_initiative);

        // The slash command carries no trigger at all.
        let cmd = write_managed(
            &command,
            Kind::Command,
            Triggers::Situational,
            Policy::refresh(),
            &mut state,
        )
        .unwrap();
        assert!(!cmd.enabled_agent_initiative, "{:?}", cmd.outcome);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The backup slot holds the only copy of a hand edit rescued by `--force`.
    /// A later routine refresh replaces shipped text with shipped text; if that
    /// took the slot, the edit would be gone one update after it was saved.
    #[test]
    fn a_rescued_hand_edit_survives_a_later_routine_refresh() {
        let dir = tmp("backup-slot");
        let path = dir.join("SKILL.md");
        let edited = "my own SKILL.md, tuned over months\n";
        std::fs::write(&path, edited).unwrap();
        let mut state = State::default();

        // `setup --force` replaces the edit and parks it in the backup slot.
        write_skill(
            &path,
            Triggers::Situational,
            Policy::install(true),
            &mut state,
        );
        assert_eq!(std::fs::read_to_string(bak(&path)).unwrap(), edited);

        // A routine refresh, twice over: shipped -> shipped, in both directions.
        // Neither is allowed to spend the slot.
        for triggers in [
            Triggers::UserOnly,
            Triggers::Situational,
            Triggers::UserOnly,
        ] {
            let outcome = write_skill(&path, triggers, Policy::refresh(), &mut state);
            assert_eq!(outcome, Outcome::Updated);
            assert_eq!(
                std::fs::read_to_string(bak(&path)).unwrap(),
                edited,
                "a routine refresh overwrote the only copy of a hand edit"
            );
        }

        // A second, genuinely different hand edit is kept too, beside the first.
        let edited_again = "a different edit, made after the first rescue\n";
        std::fs::write(&path, edited_again).unwrap();
        write_skill(
            &path,
            Triggers::Situational,
            Policy::install(true),
            &mut state,
        );
        assert_eq!(std::fs::read_to_string(bak(&path)).unwrap(), edited);
        assert_eq!(
            std::fs::read_to_string(format!("{}.1", bak(&path).display())).unwrap(),
            edited_again
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A backup of a 0600 file must not be 0644. These backups sit next to
    /// `~/.claude.json` and `~/.codex/config.toml`, which hold other tools'
    /// OAuth tokens and API keys, and nothing ever deletes them.
    #[cfg(unix)]
    #[test]
    fn a_backup_inherits_the_source_files_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp("backup-modes");

        for mode in [0o600, 0o640, 0o644] {
            let path = dir.join(format!("config-{mode:o}.json"));
            std::fs::write(&path, "{\"token\":\"secret\"}").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();

            backup(&path, b"{\"token\":\"secret\"}").unwrap();
            let actual = std::fs::metadata(bak(&path)).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                actual, mode,
                "backup of a {mode:o} file came out {actual:o}"
            );
        }

        // Re-backing up an existing, looser backup tightens it rather than
        // leaving the first run's mode in place.
        let path = dir.join("tightened.json");
        std::fs::write(&path, "loose").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        backup(&path, b"loose").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        backup(&path, b"secret now").unwrap();
        assert_eq!(
            std::fs::metadata(bak(&path)).unwrap().permissions().mode() & 0o777,
            0o600
        );

        // A managed file's backup follows the same rule.
        let skill = dir.join("SKILL.md");
        std::fs::write(&skill, "mine\n").unwrap();
        std::fs::set_permissions(&skill, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut state = State::default();
        write_skill(
            &skill,
            Triggers::Situational,
            Policy::install(true),
            &mut state,
        );
        assert_eq!(
            std::fs::metadata(bak(&skill)).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Minimal frontmatter reader for the tests: enough to prove the shipped
    /// files parse the way an agent will read them, without a YAML dependency.
    fn frontmatter_value(text: &str, key: &str) -> Option<String> {
        let rest = text.strip_prefix("---\n")?;
        let end = rest.find("\n---\n")?;
        rest[..end]
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}: ")))
            .map(str::trim)
            .map(str::to_string)
    }
}
