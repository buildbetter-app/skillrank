//! With no home directory, skillrank must write nothing at all.
//!
//! This runs the real binary rather than a function, because the defect it
//! guards was invisible from inside the process: the unit test that was
//! supposed to catch it re-derived its expected prefix from the same fallback
//! it was checking, so it cheerfully asserted that `./.claude/...` starts with
//! `./.claude`. Only an actual process, started in an actual directory, with
//! HOME actually unset, can tell the difference between "under the user's home"
//! and "under whatever the caller happened to `cd` into".
//!
//! `env -i`, systemd units, cron jobs and many container entrypoints all run
//! with no HOME, so this is an ordinary condition rather than an exotic one.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "skillrank-no-home-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run skillrank in `cwd` with every home-directory variable stripped.
fn run_homeless(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_skillrank"))
        .args(args)
        .current_dir(cwd)
        .env_remove("HOME")
        .env_remove("USERPROFILE")
        .env_remove("CODEX_HOME")
        .env_remove("SKILLRANK_HOME")
        .output()
        .expect("run skillrank")
}

/// Every file under `dir`, relative to it, sorted.
fn tree(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(
                    path.strip_prefix(dir)
                        .unwrap()
                        .display()
                        .to_string()
                        .replace('\\', "/"),
                );
            }
        }
    }
    found.sort();
    found
}

#[test]
fn setup_with_no_home_refuses_instead_of_writing_into_the_working_directory() {
    let repo = scratch("setup");
    std::fs::create_dir_all(repo.join(".claude/skills/skillrank")).unwrap();
    let checked_in = repo.join(".claude/skills/skillrank/SKILL.md");
    let original = "---\nname: skillrank\ndescription: committed by my team\n---\nour text\n";
    std::fs::write(&checked_in, original).unwrap();
    let before = tree(&repo);

    let out = run_homeless(&repo, &["setup", "--no-email"]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();

    assert!(
        !out.status.success(),
        "setup reported success with no home directory.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // The refusal has to be actionable, not just correct.
    assert!(stderr.contains("HOME"), "stderr:\n{stderr}");
    assert!(stderr.contains("--claude-config"), "stderr:\n{stderr}");
    assert!(stderr.contains("--codex-config"), "stderr:\n{stderr}");

    assert_eq!(
        tree(&repo),
        before,
        "setup created files in the working directory.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(&checked_in).unwrap(),
        original,
        "setup rewrote a file it found in the working directory"
    );
    std::fs::remove_dir_all(&repo).ok();
}

/// `--print` describes a plan without performing it, so it may still succeed —
/// but it must not describe a plan that writes into the current directory.
#[test]
fn print_with_no_home_never_advertises_a_relative_path() {
    let repo = scratch("print");
    let before = tree(&repo);

    let out = run_homeless(&repo, &["setup", "--print"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();

    for relative in ["./.claude", "./.codex", " .claude/", " .codex/"] {
        assert!(
            !stdout.contains(relative),
            "--print offered to write {relative:?}:\n{stdout}"
        );
    }
    assert!(
        stdout.contains("unavailable"),
        "--print hid the fact that it has nowhere to write:\n{stdout}"
    );
    assert_eq!(tree(&repo), before, "--print wrote something");
    std::fs::remove_dir_all(&repo).ok();
}

/// The Skill and command are only ever installed under a real home directory,
/// so `--claude-config`/`--codex-config` alone are not a licence to guess where
/// the rest goes: with those given and the file writes turned off, a homeless
/// machine can still be set up, and nothing lands in the working directory.
#[test]
fn explicit_config_paths_still_keep_everything_out_of_the_working_directory() {
    let repo = scratch("explicit");
    let elsewhere = scratch("explicit-target");
    let claude = elsewhere.join("claude.json");
    let codex = elsewhere.join("config.toml");
    let before = tree(&repo);

    let out = run_homeless(
        &repo,
        &[
            "setup",
            "--no-email",
            "--no-skill",
            "--no-command",
            "--claude-config",
            claude.to_str().unwrap(),
            "--codex-config",
            codex.to_str().unwrap(),
        ],
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(
        claude.exists() && codex.exists(),
        "explicit paths were not written.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        tree(&repo),
        before,
        "something landed in the working directory.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    std::fs::remove_dir_all(&repo).ok();
    std::fs::remove_dir_all(&elsewhere).ok();
}
