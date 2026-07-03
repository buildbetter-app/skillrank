# SkillRank

**Find, install, evaluate, and publish AI-agent skills — with real numbers.**

SkillRank is an open-source CLI and a public registry for agent skills
(`SKILL.md` packages used by Claude Code, Codex, Copilot, and others). Instead of
ranking skills by install counts, SkillRank runs reproducible **paired evals** on
your own agent and shows what a skill actually does to your **token spend, speed,
and success rate** — then lets you install, rate, review, and publish.

It works entirely on its own. The core — search, install, and local eval —
**needs no account**. It also integrates seamlessly with
[BuildBetter ZeroShot](https://buildbetter.app): install ZeroShot too and it
recommends skills from your real coding sessions and tracks realized savings.

```sh
curl -fsSL skillrank.dev | sh
```

The installer offers to also install ZeroShot — optional, and you can add it any
time. ZeroShot bundles SkillRank; SkillRank does not require ZeroShot.

## Quick start

```sh
skillrank search playwright              # browse the registry (no account)
skillrank recommend                      # suggest skills for this repo's stack
skillrank install owner/skill            # hash-verified install into .claude/skills
skillrank eval owner/skill --suite ...   # paired eval on YOUR agent; prints deltas
skillrank publish https://github.com/... # index a public skill (needs login)
```

## Commands

| Command | Account? | What it does |
|---|---|---|
| `search <query>` | no | Search the registry (filter by `--stack`, `--agent`, `--category`). |
| `show <ref>` | no | A skill's scores, security tier, and eval results by trust tier. |
| `install <ref>` | no | Verify content hash and write into the repo's skill surface; records a lockfile entry. Refuses on hash mismatch or takedown. |
| `list` / `uninstall <slug>` | no | Manage installed skills; `list` reports drift. |
| `recommend` | no | Detect this repo's stack and suggest matching skills. |
| `eval <ref> --suite <id>` | no | Run forced-mode paired trials (skill vs no-skill) on your own agent, print per-task token/success deltas, write a result bundle. `--publish` to contribute it. |
| `rate` / `review` / `publish` | yes | Contribute back. `login` stores a token; the core never needs one. |

## Using it inside Claude Code and Codex

You should never have to know a command. Register skillrank once and the agent
gets it as native tools — say *"find me a skill for playwright and install it"*
and it just works.

```sh
skillrank setup     # registers the skillrank MCP server with Claude Code + Codex
```

That writes an MCP server entry into `~/.claude.json` and `~/.codex/config.toml`
(both backed up, idempotent). Restart your agent; it now has tools
`skill_search`, `skill_recommend`, `skill_show`, `skill_install`, and
`skill_list` and calls them on its own when you talk about skills. Claude Code
prompts once to approve the tools — approve them (or pre-allow with
`{"permissions":{"allow":["mcp__skillrank"]}}` in `~/.claude/settings.json`).

**Why MCP:** the tools live in the agent's vocabulary directly, so it doesn't
guess unrelated tools and doesn't depend on skill-activation heuristics. It's the
one mechanism that works the same in Claude Code and Codex.

*Alternative / complement — a skill file.* You can also drop a `SKILL.md` that
teaches the agent about skillrank into a repo:

```sh
skillrank skill --install     # writes .claude/skills/skillrank/SKILL.md
```

Either way, skills you `install` land in `.claude/skills/` (or `.agents/skills/`)
and the agent discovers them automatically — no restart needed.

## Run a registry locally (make search work with no hosted service)

skillrank talks to a registry over HTTP (`SKILLRANK_API_URL`). Until the hosted
registry is up, run your own with one command — it serves a seed catalog of real
skills, so search / recommend / install work end to end:

```sh
skillrank serve                              # http://localhost:8899, seed catalog
export SKILLRANK_API_URL=http://localhost:8899
skillrank search "front end"                 # real results
```

To point your **agent's** MCP server at the local registry (so "find me a
front-end skill" works inside Claude Code / Codex), pass the URL to setup — it
writes it into the MCP config's env for you:

```sh
skillrank serve &                                        # keep it running
skillrank setup --api-url http://localhost:8899          # wires both agents
```

`serve --catalog <file.json>` uses your own catalog instead of the built-in seed.

## How the eval works

For each task in a suite, SkillRank runs your agent twice — once with the skill
installed (treatment) and once without (control) — against a pinned fixture repo,
then applies a **verifier that the agent never sees during the run** (verifier
isolation). It reports per-task pass-rate and token deltas locally and, if you
publish, submits a signed-attributed result bundle. Results are shown under honest
trust tiers — **Official** (reproduced by us), **Community-reported** (≥3
independent accounts, not yet reproduced), **Self-reported** — and are never mixed.

Non-Docker runs and runs off the reference agent version publish as Self-reported
only. See [`docs/`](docs) for the full methodology.

## Configuration

- `SKILLRANK_API_URL` — registry base URL (default `https://api.skillrank.dev`;
  point at a self-hosted or local registry).
- `SKILLRANK_TOKEN` — registry token for writes (or `skillrank login --token`).
- `SKILLRANK_HOME` — config dir (default `~/.skillrank`).

## Build from source

Rust (stable, edition 2021):

```sh
cargo build --release      # target/release/skillrank
cargo test
```

## Architecture

A Cargo workspace:

- `crates/skillrank-core` — **library**: registry client, lockfile, install,
  content-hash verify, stack detection, skill-surface discovery, and the eval
  harness (`runner`: forced-mode paired trials, verifier isolation, agent-usage
  parsing, bundle construction — the same code for official baselines and
  community runs). Dependency-light and agent-agnostic, so BuildBetter ZeroShot /
  the Rust `bb` CLI can embed it as a crate to provide `bb skills` from this one
  implementation.
- `crates/skillrank` — the `skillrank` **binary**: search/show/install/list/
  uninstall/recommend/eval, plus `serve` (local registry), `setup` (MCP
  registration), `mcp` (stdio MCP server), and `skill`.

The hosted registry (search, publish, reviews, leaderboards, official baselines)
is a separate service; this repo is the client + local registry + eval harness +
agent integration.

## License

MIT — see [LICENSE](LICENSE).
