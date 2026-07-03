---
name: skillrank
description: Use when the user wants to find, choose, install, evaluate, or compare agent skills — e.g. "find a skill for X", "what skills should I use in this repo", "install the playwright skill", "is this skill any good / does it actually help", "benchmark this skill". Drives the local `skillrank` CLI, which searches the public skill registry, installs skills (hash-verified) into this repo's skill surface, and runs reproducible token/speed/success evals.
---

# Using skillrank

`skillrank` is a CLI on the user's machine that finds, installs, and evaluates
agent skills. You invoke it with the Bash tool. Installed skills land in
`.claude/skills/` (or `.agents/skills/`) and become available to the agent
automatically — so "install a skill" literally means running the command below.

If `skillrank` is not found on PATH, try `~/go/bin/skillrank`, or tell the user to
install it (`go install ./cmd/skillrank` from the skillrank repo).

## When to use which command

- **"What skills should I use here?" / recommend for this repo** → `skillrank recommend`
  Detects the repo's stack and suggests matching skills. Needs no account.
- **"Find a skill for X"** → `skillrank search "<query>" [--stack <s>] [--agent claude] --json`
  Use `--json` and parse the results; present the top few with their scan tier and
  any eval lift.
- **"Tell me about / is this skill good?"** → `skillrank show <slug> --json`
  Shows scores, security tier, and eval results by trust tier (Official /
  Community-reported / Self-reported — never conflate them).
- **"Install skill X"** → `skillrank install <slug> [--yes]`
  Hash-verifies content and writes it into `.claude/skills/<slug>/SKILL.md`, then
  records `skill-registry-lock.json`. It refuses on hash mismatch or takedown.
  Do NOT pass `--yes` blindly if the scan tier is not `safe`; surface the warning
  to the user first.
- **"Does this skill actually help / benchmark it"** → `skillrank eval <slug> --suite <id> --trials 3`
  Runs paired trials (skill vs no-skill) on the user's own agent and prints
  per-task token/success deltas. It prints a cost estimate and asks to proceed;
  relay that estimate to the user before confirming. Add `--publish` only if the
  user wants to contribute the result (requires `skillrank login`).
- **"Remove skill X" / "what's installed"** → `skillrank uninstall <slug>` / `skillrank list`

## Rules

- Prefer `--json` for machine-readable output you will act on; use plain output
  only when showing the user directly.
- Reads (search/show/recommend/list) and local eval need no account. Only
  publish/rate/review require `skillrank login`.
- After installing a skill, mention that it is now active for the agent in this
  repo (auto-discovered from the skill surface) — the user does not need to
  restart anything.
- Never fabricate eval numbers. If a skill has no evals yet, say so; do not imply
  a benchmark exists.
- The registry endpoint is configurable via `SKILLRANK_API_URL`; if reads fail
  with "registry unreachable", tell the user the registry isn't reachable rather
  than guessing results.
