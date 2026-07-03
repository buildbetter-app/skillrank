//! Fixture provider (pinned-commit clone, worktree isolation) and the isolated
//! post-run verifier.

use super::{FixtureProvider, Isolation, PreparedWorkspace, Verdict, Verifier};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Prepares a pinned-commit clone of the fixture repo into a fresh temp dir per
/// trial (worktree isolation). Docker-container isolation is a planned increment.
pub struct GitFixtureProvider {
    pub git_url: String,
    pub commit: String,
    tmp_root: std::sync::Mutex<Option<std::path::PathBuf>>,
    base_checkout: std::sync::Mutex<Option<std::path::PathBuf>>,
}

static TRIAL_COUNTER: AtomicU64 = AtomicU64::new(0);

impl GitFixtureProvider {
    pub fn new(git_url: &str, commit: &str) -> Self {
        GitFixtureProvider {
            git_url: git_url.to_string(),
            commit: commit.to_string(),
            tmp_root: std::sync::Mutex::new(None),
            base_checkout: std::sync::Mutex::new(None),
        }
    }

    fn ensure_base_checkout(&self) -> Result<std::path::PathBuf, String> {
        {
            let base = self.base_checkout.lock().unwrap();
            if let Some(p) = base.as_ref() {
                return Ok(p.clone());
            }
        }
        let root = std::env::temp_dir().join(format!(
            "skillrank-fixture-{}-{}",
            std::process::id(),
            TRIAL_COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let checkout = root.join("base");

        let out = Command::new("git")
            .args(["clone", "--no-checkout", &self.git_url])
            .arg(&checkout)
            .output()
            .map_err(|e| format!("clone fixture {}: {e}", self.git_url))?;
        if !out.status.success() {
            return Err(format!(
                "clone fixture {}: {}",
                self.git_url,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        if !self.commit.trim().is_empty() {
            let co = Command::new("git")
                .args(["checkout", &self.commit])
                .current_dir(&checkout)
                .output()
                .map_err(|e| e.to_string())?;
            if !co.status.success() {
                return Err(format!(
                    "checkout fixture commit {}: {}",
                    self.commit,
                    String::from_utf8_lossy(&co.stderr).trim()
                ));
            }
        } else {
            let _ = Command::new("git")
                .args(["checkout", "HEAD"])
                .current_dir(&checkout)
                .output();
        }

        *self.tmp_root.lock().unwrap() = Some(root);
        *self.base_checkout.lock().unwrap() = Some(checkout.clone());
        Ok(checkout)
    }
}

impl FixtureProvider for GitFixtureProvider {
    fn isolation(&self) -> Isolation {
        Isolation::Worktree
    }

    fn prepare(&self, _task_id: &str) -> Result<PreparedWorkspace, String> {
        let base = self.ensure_base_checkout()?;
        let root = self
            .tmp_root
            .lock()
            .unwrap()
            .clone()
            .ok_or("fixture temp root missing")?;
        let work_dir = root.join(format!(
            "trial-{}",
            TRIAL_COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;
        copy_tree(&base, &work_dir).inspect_err(|_e| {
            let _ = std::fs::remove_dir_all(&work_dir);
        })?;
        Ok(PreparedWorkspace {
            path: work_dir.clone(),
            cleanup_root: Some(work_dir),
        })
    }
}

impl Drop for GitFixtureProvider {
    fn drop(&mut self) {
        if let Some(root) = self.tmp_root.lock().unwrap().as_ref() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

/// Whether a docker binary is on PATH.
pub fn docker_available() -> bool {
    Command::new("docker")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Recursively copy src into an existing dst directory.
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            copy_tree(&entry.path(), &target)?;
        } else if file_type.is_symlink() {
            let link_target = std::fs::read_link(entry.path()).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link_target, &target).map_err(|e| e.to_string())?;
            #[cfg(not(unix))]
            std::fs::copy(entry.path(), &target)
                .map(|_| ())
                .map_err(|e| e.to_string())?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Runs a per-task verifier command in a location OUTSIDE the workspace. The
/// verifier is materialized only inside `verify` (never in the workspace during
/// the agent run), enforcing verifier isolation structurally.
pub struct ScriptVerifier {
    /// Maps task_id -> shell command run with the workspace as $1. Exit 0 = pass.
    pub commands: HashMap<String, String>,
    /// Interpreter (default: "bash").
    pub shell: String,
}

impl ScriptVerifier {
    pub fn new(commands: HashMap<String, String>) -> Self {
        ScriptVerifier {
            commands,
            shell: "bash".to_string(),
        }
    }
}

impl Verifier for ScriptVerifier {
    fn verify(&self, working_dir: &Path, task_id: &str) -> Result<Verdict, String> {
        let command = match self.commands.get(task_id) {
            Some(c) if !c.trim().is_empty() => c,
            _ => {
                return Err(format!("no verifier for task {task_id}"));
            }
        };
        let shell = if self.shell.is_empty() {
            "bash"
        } else {
            &self.shell
        };
        // Materialize the verifier in a temp dir OUTSIDE the workspace.
        let verifier_dir = std::env::temp_dir().join(format!(
            "skillrank-verifier-{}-{}",
            std::process::id(),
            TRIAL_COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&verifier_dir).map_err(|e| e.to_string())?;
        let script_path = verifier_dir.join("verify.sh");
        std::fs::write(&script_path, command).map_err(|e| e.to_string())?;

        let status = Command::new(shell)
            .arg(&script_path)
            .arg(working_dir)
            .current_dir(working_dir)
            .status();
        let _ = std::fs::remove_dir_all(&verifier_dir);

        match status {
            Ok(s) if s.success() => Ok(Verdict {
                pass: true,
                verifier_error: false,
            }),
            Ok(_) => Ok(Verdict {
                pass: false,
                verifier_error: false,
            }),
            Err(e) => Err(e.to_string()),
        }
    }
}
