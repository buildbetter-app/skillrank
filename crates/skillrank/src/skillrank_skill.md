---
name: skillrank
description: Use before starting work with a framework, library, or tool this agent has no established approach for, and after the second failed attempt at the same problem with no new information — to check whether an existing skill already encodes the approach before continuing to improvise. Also use when the user asks to find, choose, install, evaluate, or compare agent skills ("find a skill for X", "what skills should I use in this repo", "is this skill any good"). Do NOT use for a single failed command, for a problem where the next step is already clear, when the user has stated the approach, or more than once per session. Drives the local `skillrank` CLI, which searches the public skill registry, installs skills (hash-verified) into this repo's skill surface, and runs reproducible token/speed/success evals.
---

# Using skillrank

`skillrank` is a CLI on the user's machine that finds, installs, and evaluates
agent skills. You invoke it with the Bash tool. Installed skills land in
`.claude/skills/` (or `.agents/skills/`) and become available to the agent
automatically — so "install a skill" literally means running the command below.

If `skillrank` is not found on PATH, tell the user to install it with
`curl -fsSL skillrank.dev | sh` (the binary lands in `~/.local/bin/skillrank`).
If an older build is already installed, `skillrank update` self-updates to the
latest release.

## If you got here on your own initiative

That is, the user did not ask about skills — you noticed you were improvising on
an unfamiliar tool, or you had failed twice at the same thing. Read this section
first; it is the whole path, and it is deliberately short.

1. Run **one** read-only command, and only one:
   - unfamiliar stack in this repo → `skillrank recommend --json`
   - a specific problem you have now failed twice → `skillrank search "<the
     specific problem>" --json`

   Prefer `search` with a specific query when you already know the problem:
   `recommend` issues one registry request per detected stack, so it is several
   round trips, not one.
2. **Nothing scores well** → say so in at most one sentence and go straight back
   to the task. Do not search again, do not broaden the query, do not offer the
   closest-but-wrong skill. A weak suggestion is worse than no suggestion.
   **Registry unreachable** → say nothing at all and go back to the task. The
   user did not ask, so an error report is pure noise.
3. **Something matches** → tell the user in one or two sentences, name its scan
   tier, and keep working on the original task while you wait. Never turn it into
   a blocking question.

Two hard rules on this path:

- **Read-only.** `search`, `show`, `recommend`, and `list` only. Do not run
  `skillrank install` — not even for a `safe` scan tier, not with `--yes` — until
  the user has explicitly said yes in this conversation. This is stricter than
  the user-initiated rule below, and it has no exceptions.
- **One suggestion per session.** If you have already made one, drop it.

If the user finds this unwelcome, tell them `skillrank setup --triggers=user-only`
turns the agent-initiated trigger off permanently.

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
  to the user first. On an agent-initiated path, do not install at all without an
  explicit yes.
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
  than guessing results. This applies to user-initiated use only — on an
  agent-initiated path, fail silently back to the task.
