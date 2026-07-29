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
//!   `~/.claude.json` and `~/.codex/config.toml`.
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
) -> std::io::Result<Outcome> {
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
                return Ok(Outcome::Unchanged);
            }
            let ours = hash
                .as_deref()
                .is_some_and(|h| is_shipped(h, &kind.shipped_hashes()));
            if !policy.force && !ours {
                return Ok(Outcome::UserEdited);
            }
            backup(path, &existing)?;
            write_file(path, desired)?;
            state.record_installed(path);
            Ok(Outcome::Updated)
        }
        None => {
            if state.was_installed(path) && !policy.force {
                return Ok(Outcome::UserRemoved);
            }
            if !policy.create && !policy.force {
                return Ok(Outcome::Absent);
            }
            write_file(path, desired)?;
            state.record_installed(path);
            Ok(Outcome::Created)
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
    pub result: std::io::Result<Outcome>,
}

impl Report {
    /// One line worth printing, or `None` when there is nothing to say. Silence
    /// is the right answer for the common cases (already current, never
    /// installed) — an update should not narrate its own bookkeeping.
    pub fn message(&self) -> Option<String> {
        match &self.result {
            Ok(Outcome::Created) => Some(format!("Installed {} ({})", self.label, self.show())),
            Ok(Outcome::Updated) => Some(format!("Refreshed {} ({})", self.label, self.show())),
            Ok(Outcome::Unchanged) | Ok(Outcome::Absent) | Ok(Outcome::UserRemoved) => None,
            Ok(Outcome::UserEdited) => Some(format!(
                "Left your edited {} alone ({}). A newer version ships with this release; `skillrank setup --force` replaces it (the old copy is kept as {}.skillrank-bak).",
                self.label,
                self.show(),
                self.show()
            )),
            Err(e) => Some(format!("Could not refresh {} ({}): {e}", self.label, self.show())),
        }
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

fn write_file(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

/// Copy the current bytes to `<path>.skillrank-bak` before replacing them.
pub fn backup(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let bak = format!("{}.skillrank-bak", path.display());
    std::fs::write(bak, data)
}

/// Preferences and install bookkeeping, stored as JSON in `~/.skillrank`.
///
/// Deliberately tolerant on read: a state file we cannot understand is treated
/// as no state file, exactly like the update-check cache. Losing the record
/// costs a re-created file at worst; refusing to run because of it would be
/// much worse.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub triggers: Triggers,
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

    /// The variant to write: an explicit `--triggers` wins, otherwise the
    /// recorded preference, otherwise the default. Returns the variant and
    /// whether the caller should persist a changed preference.
    pub fn resolve_triggers(&self, requested: Option<Triggers>) -> Triggers {
        requested.unwrap_or(self.triggers)
    }
}

pub fn state_path() -> Option<PathBuf> {
    config::home().ok().map(|home| home.join(STATE_FILE))
}

pub fn load_state(path: &Path) -> State {
    parse_state(&std::fs::read_to_string(path).unwrap_or_default()).unwrap_or_default()
}

pub fn save_state(path: &Path, state: &State) -> std::io::Result<()> {
    let body = serde_json::json!({
        "triggers": state.triggers.as_str(),
        "installed": state.installed,
    });
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{body}\n"))
}

/// Load state from the default location; a machine with no readable home just
/// gets defaults, which behave exactly like a fresh install.
pub fn load_default_state() -> State {
    state_path()
        .map(|path| load_state(&path))
        .unwrap_or_default()
}

/// Persist to the default location, best-effort. Nothing here is worth failing
/// a command over.
pub fn save_default_state(state: &State) {
    if let Some(path) = state_path() {
        let _ = save_state(&path, state);
    }
}

fn parse_state(body: &str) -> Option<State> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let triggers = value
        .get("triggers")
        .and_then(|v| v.as_str())
        .and_then(Triggers::parse)
        .unwrap_or_default();
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

        let first = write_managed(
            &path,
            Kind::Skill,
            Triggers::Situational,
            Policy::install(false),
            &mut state,
        )
        .unwrap();
        let second = write_managed(
            &path,
            Kind::Skill,
            Triggers::Situational,
            Policy::install(false),
            &mut state,
        )
        .unwrap();

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

        let outcome = write_managed(
            &path,
            Kind::Skill,
            Triggers::Situational,
            Policy::install(false),
            &mut state,
        )
        .unwrap();

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
            let outcome = write_managed(
                &path,
                Kind::Skill,
                Triggers::Situational,
                policy,
                &mut state,
            )
            .unwrap();
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

        let outcome = write_managed(
            &path,
            Kind::Skill,
            Triggers::Situational,
            Policy::install(false),
            &mut state,
        )
        .unwrap();

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

        let outcome = write_managed(
            &path,
            Kind::Skill,
            Triggers::Situational,
            Policy::install(true),
            &mut state,
        )
        .unwrap();

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
        write_managed(
            &path,
            Kind::Skill,
            Triggers::Situational,
            Policy::install(false),
            &mut state,
        )
        .unwrap();
        std::fs::remove_file(&path).unwrap();

        for policy in [Policy::install(false), Policy::refresh()] {
            let outcome = write_managed(
                &path,
                Kind::Skill,
                Triggers::Situational,
                policy,
                &mut state,
            )
            .unwrap();
            assert_eq!(outcome, Outcome::UserRemoved);
            assert!(!path.exists(), "absence is a choice");
        }

        let forced = write_managed(
            &path,
            Kind::Skill,
            Triggers::Situational,
            Policy::install(true),
            &mut state,
        )
        .unwrap();
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

        assert_eq!(*reports[0].result.as_ref().unwrap(), Outcome::Updated);
        assert_eq!(std::fs::read_to_string(&stale).unwrap(), SKILL_MD);
        assert!(reports[0].message().unwrap().contains("Refreshed"));
        assert_eq!(*reports[1].result.as_ref().unwrap(), Outcome::Absent);
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
            triggers: Triggers::UserOnly,
            installed: vec![path.display().to_string()],
        };

        let outcome = write_managed(
            &path,
            Kind::Skill,
            state.resolve_triggers(None),
            Policy::install(true),
            &mut state,
        )
        .unwrap();

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
            triggers: Triggers::UserOnly,
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
            triggers: Triggers::UserOnly,
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
        // A preference we do not recognise must not silently become user-only.
        std::fs::write(&path, r#"{"triggers":"whatever","installed":[1,"/a",""]}"#).unwrap();
        assert_eq!(
            load_state(&path),
            State {
                triggers: Triggers::Situational,
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
