//! The single SkillRank eval harness. Official baselines, audit re-runs, and
//! community runs all execute this code so their numbers are comparable. It runs
//! forced-mode paired trials (control = no skill, treatment = skill installed)
//! against a pinned fixture, with the verifier applied only after the agent exits
//! (verifier isolation), and produces an [`EvalBundle`].
//!
//! The orchestrator depends on traits ([`AgentRunner`], [`FixtureProvider`],
//! [`Verifier`]) so it is fully unit-testable with stubs; real implementations
//! live in [`agent`] and [`fixture`].

pub mod agent;
pub mod fixture;

use crate::types::{
    EnvironmentCell, EvalBundle, HarnessInfo, ReferenceEnv, ResolveResponse, Suite, TrialArm,
    TrialRecord,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Identifies this runner in every bundle (part of the environment cell so
/// official-vs-community results stay comparable).
pub const HARNESS_NAME: &str = "skillrank-runner";
pub const HARNESS_VERSION: &str = "0.1.0";

/// How the fixture workspace was prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Isolation {
    /// Fixture + verifier isolated in container layers.
    Docker,
    /// Pinned-commit clone in a temp dir (no container). Results are Self-reported
    /// only (cannot guarantee verifier isolation as strongly as containers).
    Worktree,
}

impl Isolation {
    pub fn as_str(self) -> &'static str {
        match self {
            Isolation::Docker => "docker",
            Isolation::Worktree => "worktree",
        }
    }
}

/// One agent invocation request.
#[derive(Debug, Clone)]
pub struct RunSpec {
    pub working_dir: PathBuf,
    pub instruction: String,
    pub model: String,
    /// Treatment arm installs the skill into the workspace surface.
    pub skill_installed: bool,
    pub skill_content: String,
    pub skill_slug: String,
    pub timeout_sec: u32,
}

/// Measured result of one agent invocation.
#[derive(Debug, Clone, Default)]
pub struct RunOutcome {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub cost_usd: Option<f64>,
    pub duration_ms: i64,
    pub turns: i64,
    pub trajectory_digest: String,
    pub agent_error: bool,
}

/// Deterministic scoring of one task after the agent run.
#[derive(Debug, Clone, Copy, Default)]
pub struct Verdict {
    pub pass: bool,
    pub verifier_error: bool,
}

/// Invokes the user's coding agent for one task in one workspace.
pub trait AgentRunner {
    /// Provider tag ("claude_code" | "codex").
    fn agent_name(&self) -> String;
    /// Reference-comparable version band.
    fn agent_version_band(&self) -> String;
    fn run_task(&self, spec: &RunSpec) -> Result<RunOutcome, String>;
}

/// A prepared per-trial workspace; the temp root is removed on drop.
pub struct PreparedWorkspace {
    pub path: PathBuf,
    pub cleanup_root: Option<PathBuf>,
}

impl Drop for PreparedWorkspace {
    fn drop(&mut self) {
        if let Some(root) = &self.cleanup_root {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

/// Prepares an isolated workspace for one task arm.
pub trait FixtureProvider {
    fn prepare(&self, task_id: &str) -> Result<PreparedWorkspace, String>;
    fn isolation(&self) -> Isolation;
}

/// Applies the (isolated, post-run) verifier to a completed workspace.
pub trait Verifier {
    fn verify(&self, working_dir: &Path, task_id: &str) -> Result<Verdict, String>;
}

/// Parameterizes an eval run.
#[derive(Debug, Clone)]
pub struct Config {
    /// Trials per arm per task.
    pub trials: u32,
    /// Model id used and recorded.
    pub model: String,
}

/// Paired per-task summary printed locally.
#[derive(Debug, Clone, Serialize)]
pub struct TaskDelta {
    pub task_id: String,
    pub control_pass_rate: f64,
    pub treatment_pass_rate: f64,
    pub pass_rate_delta: f64,
    pub control_avg_tokens: f64,
    pub treatment_avg_tokens: f64,
    pub token_delta_pct: f64,
}

/// Local paired analysis with a low-N caveat.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub deltas: Vec<TaskDelta>,
    pub trials_per_arm: u32,
    pub low_n_caveat: bool,
    pub isolation: String,
}

/// The produced bundle plus its conformance flag and local report.
pub struct EvalResult {
    pub bundle: EvalBundle,
    pub conforming: bool,
    pub report: Report,
}

/// Estimated token and USD range for a suite run (control + treatment arms).
pub fn estimate_cost(suite: &Suite, cfg: &Config) -> (i64, f64) {
    let trials = if cfg.trials == 0 { 3 } else { cfg.trials } as i64;
    let mut tokens = 0i64;
    let mut cost = 0.0f64;
    for task in &suite.tasks {
        tokens += task.est_tokens * trials * 2;
        cost += task.est_cost_usd * trials as f64 * 2.0;
    }
    (tokens, cost)
}

/// Canonicalize the run parameters that must match for two bundles to be the same
/// configuration (dedup key on ingest). Deterministic, independent of outcomes.
pub fn compute_config_hash(
    suite: &Suite,
    skill: &ResolveResponse,
    cfg: &Config,
    cell: &EnvironmentCell,
) -> String {
    let canonical = format!(
        "harness={HARNESS_NAME}/{HARNESS_VERSION}|suite={}@{}|skill={}@{}|trials={}|agent={}|band={}|model={}|os={}|isolation={}",
        suite.id, suite.version, skill.slug, skill.content_hash, cfg.trials,
        cell.agent, cell.agent_version_band, cell.model, cell.os, cell.isolation
    );
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// Execute forced-mode paired trials and build a bundle + local report.
pub fn run_eval(
    suite: &Suite,
    skill: &ResolveResponse,
    cfg: &Config,
    agent: &dyn AgentRunner,
    fixtures: &dyn FixtureProvider,
    verifier: &dyn Verifier,
) -> Result<EvalResult, String> {
    let trials = if cfg.trials == 0 { 3 } else { cfg.trials };
    if suite.tasks.is_empty() {
        return Err(format!("suite {} has no tasks", suite.id));
    }

    struct Acc {
        ctrl_pass: u32,
        ctrl_total: u32,
        treat_pass: u32,
        treat_total: u32,
        ctrl_tokens: f64,
        treat_tokens: f64,
    }
    let mut records: Vec<TrialRecord> = Vec::new();
    let mut by_task: Vec<(String, Acc)> = Vec::new();

    for task in &suite.tasks {
        let mut acc = Acc {
            ctrl_pass: 0,
            ctrl_total: 0,
            treat_pass: 0,
            treat_total: 0,
            ctrl_tokens: 0.0,
            treat_tokens: 0.0,
        };
        for arm in [TrialArm::Control, TrialArm::Treatment] {
            for i in 0..trials {
                let rec = run_one_trial(task, arm, skill, cfg, agent, fixtures, verifier).map_err(
                    |e| format!("task {} arm {} trial {}: {e}", task.id, arm.as_str(), i + 1),
                )?;
                let tokens = (rec.input_tokens + rec.output_tokens) as f64;
                let pass = rec.verdict == "pass";
                match arm {
                    TrialArm::Control => {
                        acc.ctrl_total += 1;
                        acc.ctrl_tokens += tokens;
                        if pass {
                            acc.ctrl_pass += 1;
                        }
                    }
                    TrialArm::Treatment => {
                        acc.treat_total += 1;
                        acc.treat_tokens += tokens;
                        if pass {
                            acc.treat_pass += 1;
                        }
                    }
                }
                records.push(rec);
            }
        }
        by_task.push((task.id.clone(), acc));
    }

    let cell = EnvironmentCell {
        agent: agent.agent_name(),
        agent_version_band: agent.agent_version_band(),
        model: cfg.model.clone(),
        os: std::env::consts::OS.to_string(),
        isolation: fixtures.isolation().as_str().to_string(),
    };
    let bundle = EvalBundle {
        bundle_version: 1,
        skill_slug: skill.slug.clone(),
        skill_content_hash: skill.content_hash.clone(),
        suite_id: suite.id.clone(),
        suite_version: suite.version.clone(),
        harness: HarnessInfo {
            name: HARNESS_NAME.into(),
            version: HARNESS_VERSION.into(),
        },
        environment_cell: cell.clone(),
        trials: records,
        config_hash: compute_config_hash(suite, skill, cfg, &cell),
        created_at: String::new(),
    };

    let mut deltas = Vec::new();
    for (task_id, a) in &by_task {
        let control_pass_rate = rate(a.ctrl_pass, a.ctrl_total);
        let treatment_pass_rate = rate(a.treat_pass, a.treat_total);
        let control_avg_tokens = avg(a.ctrl_tokens, a.ctrl_total);
        let treatment_avg_tokens = avg(a.treat_tokens, a.treat_total);
        let token_delta_pct = if control_avg_tokens > 0.0 {
            (treatment_avg_tokens - control_avg_tokens) / control_avg_tokens * 100.0
        } else {
            0.0
        };
        deltas.push(TaskDelta {
            task_id: task_id.clone(),
            control_pass_rate,
            treatment_pass_rate,
            pass_rate_delta: treatment_pass_rate - control_pass_rate,
            control_avg_tokens,
            treatment_avg_tokens,
            token_delta_pct,
        });
    }
    deltas.sort_by(|a, b| a.task_id.cmp(&b.task_id));

    let report = Report {
        deltas,
        trials_per_arm: trials,
        low_n_caveat: trials < 5,
        isolation: fixtures.isolation().as_str().to_string(),
    };
    let conforming = is_conforming(&suite.reference_env, &cell);
    Ok(EvalResult {
        bundle,
        conforming,
        report,
    })
}

fn run_one_trial(
    task: &crate::types::SuiteTask,
    arm: TrialArm,
    skill: &ResolveResponse,
    cfg: &Config,
    agent: &dyn AgentRunner,
    fixtures: &dyn FixtureProvider,
    verifier: &dyn Verifier,
) -> Result<TrialRecord, String> {
    // The workspace lives until the end of this function, so the verifier can run
    // against it after the agent exits.
    let workspace = fixtures.prepare(&task.id)?;
    let spec = RunSpec {
        working_dir: workspace.path.clone(),
        instruction: task.instruction.clone(),
        model: cfg.model.clone(),
        skill_installed: arm == TrialArm::Treatment,
        skill_content: skill.inline_content.clone(),
        skill_slug: skill.slug.clone(),
        timeout_sec: task.timeout_sec.max(0) as u32,
    };
    let outcome = agent.run_task(&spec)?;

    let mut rec = TrialRecord {
        task_id: task.id.clone(),
        arm,
        verdict: String::new(),
        input_tokens: outcome.input_tokens,
        output_tokens: outcome.output_tokens,
        cache_read_tokens: outcome.cache_read,
        cache_write_tokens: outcome.cache_write,
        cost_usd: outcome.cost_usd,
        duration_ms: outcome.duration_ms,
        turns: outcome.turns,
        trajectory_digest: outcome.trajectory_digest,
    };
    if outcome.agent_error {
        rec.verdict = "agent_error".into();
        return Ok(rec);
    }
    // Verifier isolation: only now, after the agent process has exited, do we
    // apply the verifier.
    match verifier.verify(&workspace.path, &task.id) {
        Err(_) => rec.verdict = "verifier_error".into(),
        Ok(v) if v.verifier_error => rec.verdict = "verifier_error".into(),
        Ok(v) if v.pass => rec.verdict = "pass".into(),
        Ok(_) => rec.verdict = "fail".into(),
    }
    Ok(rec)
}

fn is_conforming(reference: &ReferenceEnv, cell: &EnvironmentCell) -> bool {
    if cell.isolation != Isolation::Docker.as_str() {
        return false; // non-Docker runs are Self-reported only
    }
    if !reference.agent_version_band.is_empty()
        && reference.agent_version_band != cell.agent_version_band
    {
        return false;
    }
    if !reference.models.is_empty() && !reference.models.contains(&cell.model) {
        return false;
    }
    true
}

fn rate(pass: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        pass as f64 / total as f64
    }
}

fn avg(sum: f64, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        sum / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ResolveResponse, ScanTier, Suite, SuiteFixture, SuiteTask};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    struct StubFixture;
    impl FixtureProvider for StubFixture {
        fn isolation(&self) -> Isolation {
            Isolation::Worktree
        }
        fn prepare(&self, _task_id: &str) -> Result<PreparedWorkspace, String> {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "skillrank-stubfx-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("README.md"), "fixture").map_err(|e| e.to_string())?;
            Ok(PreparedWorkspace {
                path: dir.clone(),
                cleanup_root: Some(dir),
            })
        }
    }

    struct StubAgent {
        saw_verifier: AtomicBool,
    }
    impl AgentRunner for StubAgent {
        fn agent_name(&self) -> String {
            "claude_code".into()
        }
        fn agent_version_band(&self) -> String {
            "2.1".into()
        }
        fn run_task(&self, spec: &RunSpec) -> Result<RunOutcome, String> {
            // Verifier isolation check: no verify.sh in the workspace at run time.
            if spec.working_dir.join("verify.sh").exists() {
                self.saw_verifier.store(true, Ordering::SeqCst);
            }
            let mut tokens = 1000;
            if spec.skill_installed {
                std::fs::write(spec.working_dir.join("solution.txt"), "done").ok();
                tokens = 700;
            }
            Ok(RunOutcome {
                input_tokens: tokens,
                output_tokens: 100,
                turns: 2,
                duration_ms: 10,
                ..Default::default()
            })
        }
    }

    fn demo_suite() -> Suite {
        Suite {
            id: "launch/playwright".into(),
            version: "1".into(),
            fixture: SuiteFixture {
                git_url: "https://example/repo".into(),
                commit: "abc".into(),
                image: String::new(),
            },
            tasks: vec![
                SuiteTask {
                    id: "task-a".into(),
                    instruction: "do a".into(),
                    timeout_sec: 60,
                    est_tokens: 1000,
                    est_cost_usd: 0.01,
                    ..Default::default()
                },
                SuiteTask {
                    id: "task-b".into(),
                    instruction: "do b".into(),
                    timeout_sec: 60,
                    est_tokens: 1000,
                    est_cost_usd: 0.01,
                    ..Default::default()
                },
            ],
            reference_env: crate::types::ReferenceEnv {
                agent_version_band: "2.1".into(),
                models: vec!["sonnet".into()],
            },
        }
    }

    fn demo_skill() -> ResolveResponse {
        ResolveResponse {
            slug: "owner/skill".into(),
            content_hash: "sha256:aaa".into(),
            inline_content: "---\nname: skill\n---\nbody".into(),
            scan_tier: ScanTier::Safe,
            ..Default::default()
        }
    }

    #[test]
    fn arms_verifier_isolation_and_bundle() {
        let suite = demo_suite();
        let skill = demo_skill();
        let agent = StubAgent {
            saw_verifier: AtomicBool::new(false),
        };
        let fixtures = StubFixture;
        let mut commands = HashMap::new();
        commands.insert(
            "task-a".to_string(),
            "test -f \"$1/solution.txt\"".to_string(),
        );
        commands.insert(
            "task-b".to_string(),
            "test -f \"$1/solution.txt\"".to_string(),
        );
        let verifier = fixture::ScriptVerifier::new(commands);

        let cfg = Config {
            trials: 3,
            model: "sonnet".into(),
        };
        let result = run_eval(&suite, &skill, &cfg, &agent, &fixtures, &verifier).unwrap();

        assert_eq!(
            result.bundle.trials.len(),
            12,
            "2 tasks × 3 trials × 2 arms"
        );
        assert!(
            !agent.saw_verifier.load(Ordering::SeqCst),
            "verifier leaked into agent workspace"
        );
        for d in &result.report.deltas {
            assert_eq!(d.control_pass_rate, 0.0, "control should fail");
            assert_eq!(d.treatment_pass_rate, 1.0, "treatment should pass");
            assert!(d.token_delta_pct < 0.0, "treatment cheaper");
        }
        assert!(result.report.low_n_caveat, "3 trials < 5");
        assert!(!result.conforming, "worktree isolation is not conforming");
        assert_eq!(result.bundle.environment_cell.isolation, "worktree");
        assert_eq!(result.bundle.harness.name, HARNESS_NAME);
    }

    #[test]
    fn config_hash_deterministic_and_sensitive() {
        let suite = Suite {
            id: "s".into(),
            version: "1".into(),
            ..Default::default()
        };
        let skill = ResolveResponse {
            slug: "sk".into(),
            content_hash: "h".into(),
            ..Default::default()
        };
        let cell = EnvironmentCell {
            agent: "claude_code".into(),
            agent_version_band: "2.1".into(),
            model: "sonnet".into(),
            os: "macos".into(),
            isolation: "docker".into(),
        };
        let h1 = compute_config_hash(
            &suite,
            &skill,
            &Config {
                trials: 3,
                model: "sonnet".into(),
            },
            &cell,
        );
        let h2 = compute_config_hash(
            &suite,
            &skill,
            &Config {
                trials: 3,
                model: "sonnet".into(),
            },
            &cell,
        );
        assert_eq!(h1, h2, "deterministic");
        let mut cell_opus = cell.clone();
        cell_opus.model = "opus".into();
        let h3 = compute_config_hash(
            &suite,
            &skill,
            &Config {
                trials: 3,
                model: "opus".into(),
            },
            &cell_opus,
        );
        assert_ne!(h1, h3, "changes with model");
        let h4 = compute_config_hash(
            &suite,
            &skill,
            &Config {
                trials: 5,
                model: "sonnet".into(),
            },
            &cell,
        );
        assert_ne!(h1, h4, "changes with trials");
    }

    #[test]
    fn estimate_cost_works() {
        let suite = Suite {
            tasks: vec![
                SuiteTask {
                    est_tokens: 1000,
                    est_cost_usd: 0.02,
                    ..Default::default()
                },
                SuiteTask {
                    est_tokens: 2000,
                    est_cost_usd: 0.04,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let (tokens, cost) = estimate_cost(
            &suite,
            &Config {
                trials: 3,
                model: String::new(),
            },
        );
        assert_eq!(tokens, 18000);
        assert!((cost - 0.36).abs() < 0.001);
    }
}
