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

pub fn confirm(prompt: &str) -> bool {
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
        println!("{:<32} scan:{:<8} {}", item.slug, item.scan_tier, rating);
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

#[derive(Serialize)]
struct OutdatedRow {
    slug: String,
    status: String,
    installed: String,
    available: String,
}

fn short_hash(h: &str) -> String {
    h.strip_prefix("sha256:")
        .unwrap_or(h)
        .chars()
        .take(10)
        .collect()
}

/// Compare each installed skill's locked content hash to the registry's current
/// version. A skill is "outdated" when the registry has re-pinned it to a newer
/// source commit than the one we installed.
pub fn outdated(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let repo_root = core::repo_root(f.value("cwd"));
    let lock = match core::Lockfile::load(&repo_root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if lock.skills.is_empty() {
        if f.wants_json() {
            print_json(&Vec::<OutdatedRow>::new());
        } else {
            println!("No registry-installed skills in this repo.");
        }
        return 0;
    }
    let client = new_client(&f);
    let mut rows: Vec<OutdatedRow> = Vec::new();
    let (mut n_out, mut n_gone, mut n_ok) = (0u32, 0u32, 0u32);
    for e in &lock.skills {
        let (status, available) = match client.resolve(&e.slug) {
            Ok(r) if r.tombstoned => {
                n_gone += 1;
                ("unavailable", String::new())
            }
            Ok(r) if !r.content_hash.is_empty() && r.content_hash != e.computed_hash => {
                n_out += 1;
                ("outdated", r.content_hash)
            }
            Ok(r) => {
                n_ok += 1;
                ("up-to-date", r.content_hash)
            }
            Err(_) => {
                n_gone += 1;
                ("unavailable", String::new())
            }
        };
        rows.push(OutdatedRow {
            slug: e.slug.clone(),
            status: status.to_string(),
            installed: e.computed_hash.clone(),
            available,
        });
    }
    if f.wants_json() {
        print_json(&rows);
        return 0;
    }
    for r in &rows {
        if r.status == "outdated" {
            println!(
                "{:<40} OUTDATED   {} → {}",
                r.slug,
                short_hash(&r.installed),
                short_hash(&r.available)
            );
        } else {
            println!("{:<40} {}", r.slug, r.status);
        }
    }
    println!(
        "\n{n_out} outdated, {n_gone} unavailable, {n_ok} up to date (of {}).",
        lock.skills.len()
    );
    if n_out > 0 {
        println!("Upgrade with: skillrank upgrade --all");
    }
    0
}

/// Re-install skills to the registry's current version. Targets: the given
/// slugs, or every outdated skill with `--all`.
pub fn upgrade(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let repo_root = core::repo_root(f.value("cwd"));
    let client = new_client(&f);
    let lock = match core::Lockfile::load(&repo_root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    let targets: Vec<String> = if !f.positionals.is_empty() {
        f.positionals.clone()
    } else if f.bool("all") {
        let mut v = Vec::new();
        for e in &lock.skills {
            if let Ok(r) = client.resolve(&e.slug) {
                if !r.tombstoned && !r.content_hash.is_empty() && r.content_hash != e.computed_hash
                {
                    v.push(e.slug.clone());
                }
            }
        }
        v
    } else {
        eprintln!("usage: upgrade <slug>... | --all [--yes] [--surface DIR]");
        return 2;
    };

    if targets.is_empty() {
        println!("Everything is up to date.");
        return 0;
    }

    let yes = f.bool("yes") || f.bool("y");
    let (mut upgraded, mut failed) = (0u32, 0u32);
    for slug in &targets {
        let resolved = match client.resolve(slug) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{slug}: {e}");
                failed += 1;
                continue;
            }
        };
        if !core::safe_scan_tier(resolved.scan_tier) && !yes {
            println!("⚠ {} has scan tier {:?}.", slug, resolved.scan_tier);
            if !confirm("Upgrade anyway?") {
                println!("Skipped {slug}.");
                continue;
            }
        }
        match client.install(&core::InstallOptions {
            reference: slug.clone(),
            repo_root: repo_root.clone(),
            surface_override: f.value("surface").to_string(),
            now_rfc3339: None,
        }) {
            Ok(r) if r.already_exact => println!("{} already up to date.", r.slug),
            Ok(r) => {
                println!("Upgraded {} → {}", r.slug, r.skill_path);
                upgraded += 1;
            }
            Err(e) => {
                eprintln!("{slug}: {e}");
                failed += 1;
            }
        }
    }
    println!("\n{upgraded} upgraded, {failed} failed.");
    if failed > 0 {
        1
    } else {
        0
    }
}

pub fn uninstall(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let Some(slug) = f.positionals.first().cloned() else {
        eprintln!("usage: uninstall <slug> [--yes]");
        return 2;
    };
    if !f.bool("yes") && !f.bool("y") && !confirm(&format!("Remove skill {slug:?} and its files?"))
    {
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
        // Keyword search on the stack name — the registry's `stack` tag filter is
        // sparse, but a keyword query matches skill names/descriptions/tags well.
        if let Ok(resp) = client.search(&core::SearchOptions {
            query: stack.clone(),
            sort: "signals".into(),
            limit: 4,
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
    print_zeroshot_tip();
    0
}

/// One-line BuildBetter ZeroShot cross-promo, shown only after human-readable
/// (non-JSON) results so it never pollutes machine-parsed output.
pub fn print_zeroshot_tip() {
    println!("\nTip: BuildBetter ZeroShot auto-recommends skills from your real coding sessions.");
    println!("     curl -fsSL 'https://buildbetter.sh?source=skillrank-cli' | sh");
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
