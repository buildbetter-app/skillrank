//! `skillrank eval` — run a local forced-mode paired eval on the user's own agent
//! and optionally publish the result bundle.

use crate::commands::new_client;
use crate::flags::Flags;
use skillrank_core as core;
use skillrank_core::runner::{
    self,
    agent::{version_band, CliAgentRunner},
    fixture::{docker_available, GitFixtureProvider, ScriptVerifier},
    Config,
};
use std::process::Command;

pub fn run(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let Some(reference) = f.positionals.first().cloned() else {
        eprintln!("usage: eval <ref> --suite <id> [--trials N] [--agent claude|codex] [--model M] [--publish]");
        return 2;
    };
    let suite_id = f.value("suite");
    if suite_id.trim().is_empty() {
        eprintln!("error: --suite <id> is required");
        return 2;
    }
    let trials: u32 = f.value("trials").parse().unwrap_or(3).max(1);
    let provider = {
        let p = f.value("agent").trim().to_string();
        if p.is_empty() {
            detect_agent_provider()
        } else {
            p
        }
    };
    if provider != "claude" && provider != "codex" {
        eprintln!("error: could not find a supported agent CLI; install `claude` or `codex`, or pass --agent");
        return 1;
    }
    if !binary_available(&provider) {
        eprintln!("error: agent {provider:?} not found on PATH");
        return 1;
    }

    let client = new_client(&f);

    // Resolve the skill and ensure content for the treatment arm.
    let mut resolved = match client.resolve(&reference) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if resolved.inline_content.trim().is_empty() && !resolved.raw_content_url.trim().is_empty() {
        match client.fetch_raw_content(&resolved.raw_content_url) {
            Ok(c) => resolved.inline_content = c,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    }
    if resolved.inline_content.trim().is_empty() {
        eprintln!("error: registry did not provide skill content to evaluate");
        return 1;
    }

    let suite = match client.get_suite(suite_id) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let verifiers = match client.fetch_verifiers(suite_id, &suite.version) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: could not fetch verifiers for suite {suite_id}: {e}");
            return 1;
        }
    };

    let cfg = Config {
        trials,
        model: f.value("model").to_string(),
    };

    // Cost estimate + confirmation.
    let (est_tokens, est_cost) = runner::estimate_cost(&suite, &cfg);
    if !f.wants_json() {
        println!(
            "Eval plan: skill {} vs no-skill on suite {}@{}",
            resolved.slug, suite.id, suite.version
        );
        println!(
            "  agent: {provider} | model: {} | {trials} trials/arm | {} tasks × 2 arms",
            or_dash(cfg.model.as_str()),
            suite.tasks.len()
        );
        println!(
            "  estimated: ~{} tokens, ~${:.2} on YOUR agent subscription",
            human_int(est_tokens),
            est_cost
        );
        if !docker_available() {
            println!("  note: Docker not detected → worktree isolation; results publish as Self-reported.");
        }
        if !f.bool("yes") && !f.bool("y") && !crate::commands::confirm("Proceed?") {
            println!("Aborted.");
            return 1;
        }
    }

    let agent = CliAgentRunner {
        provider: provider.clone(),
        binary: provider.clone(),
        version: detect_agent_version(&provider),
    };
    let fixtures = GitFixtureProvider::new(&suite.fixture.git_url, &suite.fixture.commit);
    let verifier = ScriptVerifier::new(verifiers);

    let result = match runner::run_eval(&suite, &resolved, &cfg, &agent, &fixtures, &verifier) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let mut bundle = result.bundle;
    bundle.created_at = core::install::now_rfc3339();

    let bundle_path = write_local_bundle(&bundle);

    if f.wants_json() {
        let out = serde_json::json!({
            "bundle": bundle,
            "report": result.report,
            "conforming": result.conforming,
            "bundlePath": bundle_path.clone().unwrap_or_default(),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        print_report(&result.report);
        if let Some(p) = &bundle_path {
            println!("\nBundle written: {p}");
        }
    }

    if f.bool("publish") {
        match client.submit_bundle(&bundle) {
            Ok(resp) => {
                if !f.wants_json() {
                    let tier = if resp.tier_state.is_empty() {
                        "self_reported"
                    } else {
                        &resp.tier_state
                    };
                    println!(
                        "Published (tier: {tier}{})",
                        conform_note(result.conforming)
                    );
                }
            }
            Err(e) => {
                eprintln!("publish failed: {e}");
                return 1;
            }
        }
    }
    if !f.wants_json() {
        crate::commands::print_zeroshot_tip();
    }
    0
}

fn print_report(report: &runner::Report) {
    println!(
        "\nResults ({} trials/arm, {} isolation):",
        report.trials_per_arm, report.isolation
    );
    for d in &report.deltas {
        println!(
            "  {:<24} pass {:.0}%→{:.0}% ({:+.0} pp), tokens {:+.1}%",
            d.task_id,
            d.control_pass_rate * 100.0,
            d.treatment_pass_rate * 100.0,
            d.pass_rate_delta * 100.0,
            d.token_delta_pct
        );
    }
    if report.low_n_caveat {
        println!("  (low N: <5 trials/arm — treat deltas as directional, not significant)");
    }
}

fn conform_note(conforming: bool) -> String {
    if conforming {
        String::new()
    } else {
        "; not on reference environment → not eligible for Community-reported aggregation".into()
    }
}

fn write_local_bundle(bundle: &core::EvalBundle) -> Option<String> {
    let home = core::config::home().ok()?;
    let dir = home.join("bundles");
    std::fs::create_dir_all(&dir).ok()?;
    let name = format!(
        "{}_{}_{}.json",
        sanitize(&bundle.skill_slug),
        sanitize(&bundle.suite_id),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    let path = dir.join(name);
    let json = serde_json::to_string_pretty(bundle).ok()?;
    std::fs::write(&path, json).ok()?;
    Some(path.to_string_lossy().to_string())
}

fn detect_agent_provider() -> String {
    for candidate in ["claude", "codex"] {
        if binary_available(candidate) {
            return candidate.to_string();
        }
    }
    String::new()
}

fn binary_available(provider: &str) -> bool {
    Command::new(provider)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn detect_agent_version(provider: &str) -> String {
    let Ok(out) = Command::new(provider).arg("--version").output() else {
        return "unknown".to_string();
    };
    let s = String::from_utf8_lossy(&out.stdout);
    for field in s.split_whitespace() {
        if field
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            return field.to_string();
        }
    }
    let _ = version_band; // keep import meaningful for downstream
    s.trim().to_string()
}

fn or_dash(s: &str) -> String {
    if s.trim().is_empty() {
        "(agent default)".to_string()
    } else {
        s.to_string()
    }
}

fn human_int(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}
