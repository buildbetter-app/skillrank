//! `skillrank skill` — print or install the SKILL.md that teaches an agent to use
//! skillrank. Embedded so the installed binary can write it into any repo.

use crate::flags::Flags;
use crate::managed;
use serde_json::json;
use skillrank_core as core;

pub fn run(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    // Honour the recorded trigger preference here too. An off switch that only
    // covers `setup` is not an off switch.
    let skill_md = managed::load_default_state()
        .resolve_triggers(None)
        .skill_text();
    if !f.bool("install") {
        print!("{skill_md}");
        if !f.wants_json() {
            eprintln!("\n(Run `skillrank skill --install` to add this to .claude/skills so your agent uses skillrank automatically.)");
        }
        return 0;
    }
    let repo_root = core::repo_root(f.value("cwd"));
    let (surface_rel, surface_abs) = core::resolve_surface(&repo_root, f.value("surface"));
    let dir = surface_abs.join("skillrank");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("error: {e}");
        return 1;
    }
    let path = dir.join("SKILL.md");
    if let Err(e) = std::fs::write(&path, skill_md) {
        eprintln!("error: {e}");
        return 1;
    }
    let rel = format!("{surface_rel}/skillrank/SKILL.md");
    if f.wants_json() {
        println!("{}", json!({ "skillPath": rel }));
        return 0;
    }
    println!("Installed the skillrank skill → {rel}");
    println!("Your agent will now use skillrank automatically when you ask it to find, install, or evaluate skills.");
    0
}
