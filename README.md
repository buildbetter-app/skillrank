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
curl -fsSL https://skillrank.dev/install.sh | sh
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

## Using it inside Claude Code (or Codex)

skillrank is a CLI, not a skill. It *installs* skills into `.claude/skills/` (or
`.agents/skills/`), where your agent discovers them automatically — so
`skillrank install <slug>` is all it takes to give Claude a new skill.

To let the agent drive skillrank *for you* — "find me a good Playwright skill and
install it", "does this skill actually help?" — install the bundled skillrank
skill once per repo:

```sh
skillrank skill --install     # writes .claude/skills/skillrank/SKILL.md
```

Now when you ask Claude Code to find, install, or evaluate skills, it runs the
right `skillrank` commands itself. (`skillrank skill` with no flag prints the
SKILL.md.) Installed skills need no restart — the agent picks them up on its next
run.

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

```sh
go build -o skillrank ./cmd/skillrank   # Go 1.23+
go test ./...
```

## Architecture

- `cmd/skillrank` — the binary.
- `internal/registry` — registry client, lockfile, install, content-hash verify,
  stack detection.
- `internal/registry/runner` — the eval harness (fixture lifecycle, paired trials,
  verifier isolation, agent usage parsing, bundle construction). The **same
  harness** runs official baselines, audits, and community evals.
- `internal/{command,config,api,skills}` — the small, dependency-free CLI harness.

The hosted registry (search, publish, reviews, leaderboards, official baselines)
is a separate service; this repo is the client + the local eval harness. ZeroShot
embeds this module to provide `bb skills`.

## License

MIT — see [LICENSE](LICENSE).
