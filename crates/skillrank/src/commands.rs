//! Read + install command handlers. Each returns a process exit code.

use crate::flags::Flags;
use serde::Serialize;
use skillrank_core as core;
use std::io::Write;

pub fn new_client(f: &Flags) -> core::Client {
    let override_url = f.value("api-base-url");
    core::Client::new(if override_url.is_empty() {
        None
    } else {
        Some(override_url)
    })
}

fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("error serializing output: {e}"),
    }
}

fn confirm(prompt: &str) -> bool {
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
}

pub fn search(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let query = f.positionals.join(" ");
    let client = new_client(&f);
    let limit = f.value("limit").parse::<u32>().unwrap_or(20);
    let resp = match client.search(&core::SearchOptions {
        query,
        stack: f.value("stack").to_string(),
        agent: f.value("agent").to_string(),
        category: f.value("category").to_string(),
        sort: f.value("sort").to_string(),
        limit,
        ..Default::default()
    }) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if f.wants_json() {
        print_json(&resp);
        return 0;
    }
    if resp.items.is_empty() {
        println!("No skills matched.");
        return 0;
    }
    for item in &resp.items {
        let rating = match item.rating_average {
            Some(avg) => format!("{avg:.1}★ ({})", item.rating_count),
            None => "—".to_string(),
        };
        println!(
            "{:<32} scan:{:<8} {}",
            item.slug, item.scan_tier, rating
        );
        if !item.summary.is_empty() {
            println!("    {}", truncate(&item.summary, 100));
        }
    }
    0
}

pub fn show(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let Some(reference) = f.positionals.first() else {
        eprintln!("usage: show <ref>");
        return 2;
    };
    let (slug, _) = core::split_ref(reference);
    let client = new_client(&f);
    let detail = match client.show(&slug) {
        Ok(d) => d,
        Err(e) if e.is_not_found() => {
            eprintln!("skill {slug:?} not found");
            return 1;
        }
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if f.wants_json() {
        print_json(&detail);
        return 0;
    }
    let s = &detail.summary;
    println!("{}", s.slug);
    if !s.summary.is_empty() {
        println!("  {}", s.summary);
    }
    println!("  source: {} ({})", s.source_url, s.source_type);
    println!("  scan:   {}", s.scan_tier);
    if !s.stacks.is_empty() {
        println!("  stacks: {}", s.stacks.join(", "));
    }
    if detail.eval_cells.is_empty() {
        println!("  evals:  none yet");
    } else {
        println!("  evals:");
        for cell in &detail.eval_cells {
            let lift = cell
                .success_lift_pct
                .map(|v| format!("{v:+.1}%"))
                .unwrap_or_else(|| "—".into());
            println!(
                "    [{:?}] {}/{} on {}: lift {} (n={} accts, {} trials)",
                cell.tier, cell.agent, cell.model, cell.suite, lift, cell.n_accounts, cell.n_trials
            );
        }
    }
    0
}

pub fn install(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let Some(reference) = f.positionals.first().cloned() else {
        eprintln!("usage: install <ref> [--surface DIR] [--yes]");
        return 2;
    };
    let client = new_client(&f);
    let repo_root = core::repo_root(f.value("cwd"));

    let resolved = match client.resolve(&reference) {
        Ok(r) => r,
        Err(e) if e.is_not_found() => {
            eprintln!("skill {reference:?} not found in the registry");
            return 1;
        }
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let yes = f.bool("yes") || f.bool("y");
    if !core::safe_scan_tier(resolved.scan_tier) {
        if yes {
            eprintln!(
                "warning: installing {} despite scan tier {:?} (--yes)",
                resolved.slug, resolved.scan_tier
            );
        } else {
            println!(
                "⚠ {} has scan tier {:?} (not verified safe).",
                resolved.slug, resolved.scan_tier
            );
            if !confirm("Install anyway?") {
                println!("Aborted.");
                return 1;
            }
        }
    }

    let result = match client.install(&core::InstallOptions {
        reference,
        repo_root,
        surface_override: f.value("surface").to_string(),
        now_rfc3339: None,
    }) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if f.wants_json() {
        print_json(&result);
        return 0;
    }
    if result.already_exact {
        println!(
            "{} already installed at {} (up to date).",
            result.slug, result.skill_path
        );
    } else {
        println!(
            "Installed {} → {} (scan: {})",
            result.slug, result.skill_path, result.scan_tier
        );
    }
    0
}

pub fn list(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let repo_root = core::repo_root(f.value("cwd"));
    let mut rows = match core::list_installed(&repo_root) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if f.wants_json() {
        print_json(&rows);
        return 0;
    }
    if rows.is_empty() {
        println!("No registry-installed skills in this repo.");
        return 0;
    }
    rows.sort_by(|a, b| a.slug.cmp(&b.slug));
    for r in &rows {
        println!("{:<32} {:<16} {}", r.slug, r.state, r.skill_path);
    }
    0
}

pub fn uninstall(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let Some(slug) = f.positionals.first().cloned() else {
        eprintln!("usage: uninstall <slug> [--yes]");
        return 2;
    };
    if !f.bool("yes") && !f.bool("y") && !confirm(&format!("Remove skill {slug:?} and its files?")) {
        println!("Aborted.");
        return 1;
    }
    let repo_root = core::repo_root(f.value("cwd"));
    match core::uninstall(&repo_root, &slug) {
        Ok(path) => {
            println!("Removed {slug} ({path})");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

pub fn recommend(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let repo_root = core::repo_root(f.value("cwd"));
    let detected = core::detect_stack(&repo_root);
    let client = new_client(&f);

    #[derive(Serialize)]
    struct Recommendation {
        detected: core::DetectedStack,
        skills: Vec<core::SkillSummary>,
    }
    let mut rec = Recommendation {
        detected: detected.clone(),
        skills: Vec::new(),
    };
    let mut seen = std::collections::HashSet::new();
    for stack in &detected.stacks {
        if let Ok(resp) = client.search(&core::SearchOptions {
            stack: stack.clone(),
            sort: "signals".into(),
            limit: 5,
            ..Default::default()
        }) {
            for item in resp.items {
                if seen.insert(item.slug.clone()) {
                    rec.skills.push(item);
                }
            }
        }
    }
    if f.wants_json() {
        print_json(&rec);
        return 0;
    }
    if detected.stacks.is_empty() {
        println!("Could not detect a stack in this repo. Try `skillrank search <query>`.");
        return 0;
    }
    println!("Detected stack: {}", detected.stacks.join(", "));
    if rec.skills.is_empty() {
        println!("No matching skills found in the registry yet.");
        return 0;
    }
    println!("Recommended skills:");
    for item in &rec.skills {
        println!(
            "  {:<32} scan:{:<8} {}",
            item.slug,
            item.scan_tier,
            truncate(&item.summary, 80)
        );
    }
    println!("\nInstall one with: skillrank install <slug>");
    0
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.replace('\n', " ");
    let s = s.trim();
    if s.chars().count() <= n {
        return s.to_string();
    }
    let truncated: String = s.chars().take(n.saturating_sub(1)).collect();
    format!("{truncated}…")
}
