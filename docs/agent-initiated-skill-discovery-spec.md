# Agent-Initiated Skill Discovery Spec

This spec defines desired behavior and acceptance criteria. It is not an execution plan.

Status: draft, unimplemented. Written 2026-07-29 against skillrank 0.1.4.

## Purpose

`skillrank setup` already installs a Skill into Claude Code and Codex, registers an MCP
server with both, and writes a slash command. All of that plumbing works. The Skill has
nevertheless been invoked **zero times in 2,569 sessions**, because its `description`
frontmatter only describes situations in which *the user* asks about skills.

This spec changes the trigger, not the plumbing. It defines a description that lets an
agent recognise *its own* situation — repeated failure on one problem, an unfamiliar
tool, a task type it has no established approach for — and the guardrails that keep that
from turning skillrank into nagware.

The outcome that should change: skillrank's Skill invocation rate goes from 0% of
sessions to the 0.5–1.5% band where genuinely useful situational skills actually live on
this same machine, without exceeding a 3% fire rate or producing installs the user
reverts.

## Problem

### The measured gap

From a scan of `~/.claude/projects` (3,285 `.jsonl` session transcripts, this machine,
2026-07-29):

| Signal | Count |
| --- | --- |
| Sessions with skillrank MCP tools in context | 2,219 |
| Sessions with any `mcp__skillrank__*` tool call | 1 |
| Total skillrank MCP tool calls, ever | 3 (2× `skill_search`, 1× `skill_recommend`) |
| Sessions with a `Skill(skillrank)` invocation | **0** |

The one session that called the MCP tools was the team building skillrank. Roughly 700
sessions loaded skillrank for every one that used it, and the Skill surface specifically
has never fired at all.

Reproduce with:

```
cd ~/.claude/projects
grep -rl '"name":"Skill","input":{"skill":"skillrank"' --include='*.jsonl' . | wc -l
grep -rl '"name":"mcp__skillrank__' --include='*.jsonl' . | wc -l
```

### Root cause: the description only describes user requests

`crates/skillrank/src/skillrank_skill.md` frontmatter, shipped verbatim by
`setup::ensure_skill` to `~/.claude/skills/skillrank/SKILL.md` and
`~/.codex/skills/skillrank/SKILL.md` (verified identical to the copy on disk):

> `description: Use when the user wants to find, choose, install, evaluate, or compare
> agent skills — e.g. "find a skill for X", "what skills should I use in this repo",
> "install the playwright skill", …`

Every clause is user-initiated. Claude Code loads a skill by matching its description
against the current situation, so this can only fire once the user already knows
skillrank exists and has decided to use it — precisely the population that does not need
a reminder. An agent that is stuck, or working with a library it is guessing at, matches
none of these phrases.

The same is true of the MCP layer: `mcp.rs` advertises `skill_search` as *"Use when the
user asks to find a skill for something"* and `skill_recommend` as *"Use when the user
asks 'what skills should I use here'"*. The MCP `instructions` string in the initialize
response is a pure capability blurb. Both surfaces encode the same assumption.

### The natural experiment that isolates the cause

The same machine, the same agent, and the same skill surface contain skills whose
descriptions *are* situational. Their invocation rates:

| Skill | Description shape | Sessions present | Sessions invoked | Rate |
| --- | --- | --- | --- | --- |
| `playwright-cli` | capability/noun ("Automate browser interactions…") | 2,511 | 38 | 1.51% |
| `superpowers:brainstorming` | "You MUST use this **before any creative work**…" | 3,048 | 20 | 0.66% |
| `superpowers:systematic-debugging` | "Use when **encountering any bug, test failure, or unexpected behavior**, before proposing fixes" | 3,047 | 16 | 0.52% |
| `find-skills` | user-initiated ("when they ask…") | 3,022 | 1 | 0.03% |
| `skillrank` | user-initiated | 2,569 | **0** | **0.00%** |

Total `Skill` invocations across all skills, all sessions: 186.

Two independent skills whose job is skill discovery (`skillrank` and `find-skills`),
both user-initiated, produce 1 invocation between them across ~5,500 session-exposures.
Three situational skills in the same surface produce 74. That is strong evidence the
failure is the *category of trigger*, not skillrank-specific wording weakness.

It is not proof. The skills also differ in usefulness, and "the agent needs a registry
skill" is a rarer real situation than "the agent hit a bug". Treat the rewrite as a
well-supported hypothesis with a measurable outcome, not a certain fix. See
[Acceptance Criteria](#acceptance-criteria) for the rollback condition.

### Why the descriptions that work, work

`brainstorming` and `systematic-debugging` share a structure the current skillrank
description lacks:

1. **A situation the agent can observe about itself** ("encountering any bug", "creating
   features"), not a request it must receive.
2. **A temporal gate that names the moment** ("before proposing fixes", "before writing
   implementation code"). This binds the skill to a decision point the agent already
   pauses at, rather than asking it to spontaneously reconsider mid-flow.

The gate is doing most of the work. A skill that says "use when X is true" competes with
everything else in context at every turn; a skill that says "before you do Y" attaches
to a specific, recurring branch in the agent's own loop.

### Scale of the attention budget

This machine's global skill surface holds 51 skills, plus plugin skills, plus MCP tool
descriptions. Every description competes for the same attention. A rewrite must earn its
place against 50+ rivals, which caps how much can be achieved by wording alone and makes
over-firing a shared-resource cost, not just a skillrank cost.

## Goals

1. Rewrite the shipped Skill description so an agent can recognise its own situation as
   matching, without the user mentioning skills.
2. Bound how eagerly the new trigger fires, in the description text itself, using
   explicit negative gates — because the body is not read until the skill has already
   fired, so all gating must live in the description.
3. Separate *triggering* from *acting*: an agent-initiated fire may search and report;
   it may not install without explicit user confirmation.
4. Give the user a discoverable, permanent off switch that survives re-running `setup`
   and `update`.
5. Define success and failure numerically, observable from local transcripts and the
   existing lockfile, with no new telemetry.
6. Deliver the new trigger to the ~existing installed base, which currently has no
   mechanism to receive it.

## Non-Goals

- **No new detection machinery in skillrank.** No hooks, no transcript watcher, no
  background daemon, no session-state tracking in the CLI. The agent's own judgement is
  the detector. Anything else is a different, larger spec.
- **No changes to search, install, eval, scan tiers, or the registry API.**
- **Not porting ZeroShot's friction detection into skillrank.** ZeroShot already
  analyses local coding sessions and proposes skills after the fact. Extending that to
  propose *registry installs* rather than *new authored skills* is valuable and out of
  scope here. This spec is the in-session half; ZeroShot is the after-the-fact half.
- **No telemetry.** No phone-home on skill invocation. Measurement is local-only.
- **Not making the agent install skills autonomously.** Explicitly excluded; see R5.
- **Not rewriting the slash command** (`skillrank_command.md`). It is explicitly
  user-initiated and correctly so.

## Users and Actors

| Actor | Role |
| --- | --- |
| **Coding agent** (Claude Code, Codex) | Reads the skill description every turn; decides whether its own situation matches. The primary audience for the description text. |
| **skillrank user** (installed the CLI) | Owns the machine and the repo. Must be able to turn this off in one step and must never find third-party code installed without having said yes. |
| **Repo owner / teammate** (may not have installed skillrank) | May share a repo with a skillrank user. Must never receive an unrequested commit to their `CLAUDE.md` / `AGENTS.md`. Constrains R6. |
| **`skillrank setup`** | Writes the Skill, command, and MCP registration. Currently the only writer of skill text. |
| **`skillrank update`** | Swaps the binary. Verified *not* to re-run setup — the bootstrap gap in R7. |
| **`install.sh`** | Calls `skillrank setup` on first install (lines 109–127). The only automatic setup path today. |

## Requirements

### R1 — Rewritten Skill description

`crates/skillrank/src/skillrank_skill.md` frontmatter MUST be replaced with a
description that (a) retains the existing user-initiated triggers, (b) adds
agent-initiated triggers, and (c) carries explicit negative gates.

Proposed text:

```yaml
description: Use before starting work with a framework, library, or tool this agent has
  no established approach for, and after the second failed attempt at the same problem
  with no new information — to check whether an existing skill already encodes the
  approach before continuing to improvise. Also use when the user asks to find, choose,
  install, evaluate, or compare agent skills ("find a skill for X", "what skills should
  I use in this repo", "is this skill any good"). Do NOT use for a single failed
  command, for a problem where the next step is already clear, when the user has stated
  the approach, or more than once per session. Drives the local `skillrank` CLI, which
  searches the public skill registry, installs skills (hash-verified) into this repo's
  skill surface, and runs reproducible token/speed/success evals.
```

Clause-by-clause rationale, and its limits:

| Clause | Why it should match a real situation | Honest weakness |
| --- | --- | --- |
| "**before starting work with a framework, library, or tool this agent has no established approach for**" | Strongest clause. It fires at task start — the same decision point that makes `brainstorming` (0.66%) and TDD fire — where the agent is already planning rather than executing, and a detour is cheapest. "No established approach" is a state a model can genuinely assess. | Models are reluctant to admit unfamiliarity and tend to proceed confidently. Expect this to under-fire relative to how often it is true. |
| "**after the second failed attempt at the same problem with no new information**" | Describes a state that is literally present in the agent's own context window, so the match is against visible evidence rather than a memory. "With no new information" is the load-bearing part: it separates productive iteration (each attempt narrows the cause) from thrashing. | Models are unreliable at self-counting attempts. This is a *recognition cue*, not a counter — it works when the agent re-reads the skill list while re-planning, and does nothing if it never re-plans. Do not expect precision here. Chosen "second" over "third" because a third-attempt gate combined with poor self-counting would likely never fire at all. |
| "**before continuing to improvise**" | The temporal gate. Matches the structural pattern shared by every situational skill that actually fires on this machine. Binds the skill to the moment before the next attempt, not to a general condition. | None material; this is the cheapest and highest-leverage phrase in the description. |
| "**Do NOT** … single failed command / next step already clear / user stated the approach / more than once per session" | Negative gates are the only eagerness control available, because the description is all that is loaded before the decision. Precedent exists: Anthropic's own `claude-api` skill ships an explicit "SKIP only when…" clause that overrides its triggers. | Negative gates are advisory. They reduce fire rate; they do not enforce a ceiling. The enforceable ceiling is R5. |
| Retained user-initiated clauses | They are correct and they cost nothing. They are simply insufficient alone. | — |

**What a description can and cannot cause.** It can make a skill *eligible* to match at a
decision point. It cannot force a re-plan, cannot count anything, cannot override the
user's active instruction, and cannot win attention against 50+ competing descriptions
in every session. Anyone reading this spec should expect a rate in the low single-digit
percent at best — matching `playwright-cli`'s 1.51%, not exceeding it.

### R2 — Rewritten MCP tool and instruction text

The MCP surface reaches 2,219 sessions and is server-owned, so it costs the user
nothing. It MUST carry the same situational framing:

- `initialize.instructions` (`mcp.rs:120`) MUST describe when to reach for skillrank,
  not only what it does. Required content: check the registry before improvising on an
  unfamiliar tool or after repeated failure; do not install without user confirmation.
- `skill_search.description` and `skill_recommend.description` (`mcp.rs:319`, `:334`)
  MUST drop "Use when the user asks…" as their sole trigger and add the agent-initiated
  situation.
- `skill_install.description` MUST state that installation requires user confirmation
  when the agent reached the tool on its own initiative, not only when the scan tier is
  unsafe.

Honest note: MCP instructions were present in all 2,219 sessions and yielded 3 calls.
Their measured effect so far is approximately zero. They are included because they are
free and server-owned, not because they are expected to carry the result.

### R3 — Skill body: cheapest-first ordering

The body (`skillrank_skill.md`, ~495 words) is loaded only after the skill fires, so its
cost is paid only on a real match. It MUST open with an agent-initiated triage path
placed *before* the existing per-command reference:

1. If reached on the agent's own initiative, run **one** read-only command —
   `skillrank recommend --json` when the situation is "unfamiliar stack in this repo",
   or `skillrank search "<the specific problem>" --json` when the situation is a
   specific repeated failure.
2. If nothing scores well, say so in one sentence and return to the original task. Do
   not search a second time, do not broaden the query.
3. If something matches, surface it to the user in one or two sentences with its scan
   tier, and continue the original task while awaiting an answer. Do not block.

Note that `commands::recommend` (`commands.rs:430`) issues one registry search *per
detected stack*, so a `recommend` call is several HTTP requests, not one. The body must
prefer `search` with a specific query for the repeated-failure path.

### R4 — Cost of being wrong, and where the line sits

The eagerness dial, measured against the same 3,285 transcripts:

| Trigger condition | Sessions matching | Share |
| --- | --- | --- |
| Any error tool result | 843 | 25.7% |
| ≥5 error tool results | 94 | 2.9% |
| ≥8 error tool results | 49 | 1.5% |
| ≥15 error tool results | 27 | 0.8% |

A trigger on "any failure" would fire in a quarter of all sessions. That is unambiguous
nagware. A trigger on genuine thrash lands at 1–3%, the same order as the best-firing
situational skills. **The line sits at ≤3% of sessions.**

The two failure directions are not symmetric:

- **Never fires** (today): cost is the status quo — a tool that is installed, correct,
  and invisible. Recoverable at any time; nothing is lost but the opportunity.
- **Fires too eagerly**: cost is a mid-task detour of roughly 1–3k tokens (skill body
  plus one search) *plus* attention displacement at the moment the user most wants
  focus. Worse, it is not recoverable: a tool that interrupts with mediocre suggestions
  gets muted, and muting is permanent. The user does not re-enable it later to check
  whether it improved.

Because the downside is asymmetric and irreversible, the trigger is deliberately tuned
to under-fire. Ship the conservative version; loosen only against measured data.

### R5 — Precision bar: an agent-initiated fire may not mutate the repo

This is the enforceable half of R4, and the only guardrail that does not depend on the
model following prose.

- When the skill is reached without the user having asked about skills, the agent MUST
  restrict itself to read-only operations (`search`, `show`, `recommend`, `list`).
- `skill_install` / `skillrank install` MUST NOT be called on an agent-initiated path
  without an explicit user "yes" in the conversation. This is stricter than today's
  rule, which only requires confirmation when the scan tier is not `safe`.
- At most one suggestion per session, one sentence, presented alongside continued
  progress on the original task — never as a blocking question.

Rationale: this bounds the worst case of over-firing to wasted tokens. Without it, the
worst case is unrequested third-party code in the user's repo, which is a different
class of harm and would justify a far more conservative trigger.

### R6 — Project-level instruction: opt-in only

A skill description alone is probably **not** sufficient for the repeated-failure
trigger, for the reason given in R1: mid-task re-consultation of the skill list is not
guaranteed. A project-level instruction line sits in a different part of context and is
re-read more reliably.

That said, writing into a user's project files is writing into someone else's repo — it
lands in git, in a PR, and in a teammate's review. Therefore:

- `skillrank setup` MUST NOT write to `CLAUDE.md`, `AGENTS.md`, or any repo file **by
  default**, and `install.sh` MUST NOT pass any flag that causes it to.
- An explicit, separately-invoked `skillrank setup --project` MAY write **exactly one**
  delimited block to an existing `CLAUDE.md` or `AGENTS.md` in the current repo root:

  ```markdown
  <!-- skillrank:begin -->
  When you have failed twice at the same problem with no new information, or are about
  to work with a tool this repo has no established approach for, check `skillrank
  search "<problem>"` for an existing skill before improvising further. Suggest, do not
  install — installs need an explicit yes.
  <!-- skillrank:end -->
  ```

- It MUST NOT create the file if absent — refuse with a message instead. Creating
  `CLAUDE.md` in a repo that never had one is a visible, unrequested change.
- It MUST refuse if the working tree is dirty, so the user can review and revert the
  edit as an isolated diff.
- It MUST print the exact diff and the removal command on success.
- `skillrank setup --project --remove` MUST delete the block by marker and leave the
  rest of the file byte-identical.
- Re-running `--project` MUST replace the existing block, never append a second one.

Recommendation: ship R1–R5 first and measure. Only reach for R6 if the description
rewrite alone lands below the success band. A repo-file write is a trust cost that
should be spent only against evidence that it is needed.

### R7 — Bootstrap: delivering the new trigger to existing installs

Verified current behavior:

- `setup::ensure_skill` → `write_owned_file` overwrites `SKILL.md` unconditionally, with
  **no backup** (`backup()` is called only for the MCP config files).
- `update::run` swaps the binary and does nothing else. It never re-runs setup.
- `setup` is invoked automatically only by `install.sh` at first install.

So every existing user has the old description permanently, and nothing will ever
replace it. This must be fixed or the rewrite reaches nobody.

- `skillrank update` MUST, after a successful binary swap, compare the on-disk
  `SKILL.md` at each known agent path against the embedded text and refresh it when
  they differ.
- Before overwriting, it MUST compare against a set of hashes of previously shipped
  versions. If the on-disk content matches none of them, the file is user-edited: leave
  it alone, print a one-line notice naming the path, and continue.
- Fix the unconditional-overwrite defect in `ensure_skill` the same way, so the existing
  `skillrank setup` path stops silently destroying user edits.
- The existing startup update check (`update_check.rs`) already prints at most one
  stderr line, only on a TTY, never in CI, and honours `SKILLRANK_NO_UPDATE_CHECK`. For
  users who do not run `update`, it MAY add one line pointing at `skillrank setup`
  **once**, recorded in `~/.skillrank/`. It MUST NOT repeat.

### R8 — The off switch

- `skillrank setup --triggers=user-only` MUST install the pre-rewrite, user-initiated
  description instead of the new one.
- That choice MUST be recorded in `~/.skillrank/` (the state dir already exists and
  holds `update-check.json`) and MUST be honoured by every later `setup` and `update`,
  so turning it off once is permanent.
- If the user deletes `~/.claude/skills/skillrank/`, `setup` and `update` MUST NOT
  silently re-create it. Absence is a choice; treat it as one.
- `skillrank setup --print` MUST show which trigger variant would be written.
- The off switch MUST be named in the skill body and in the one-time notice from R7, so
  a user who finds it annoying learns how to stop it at the moment they are annoyed —
  not by going to look for documentation.

## Acceptance Criteria

All observable from a local transcript scan plus `skill-registry-lock.json`. No
telemetry. Measurement window: 60 days after the rewrite reaches an install, or 300 new
sessions, whichever comes first.

**A1 — It fires at all.** ≥1 `Skill(skillrank)` invocation, or ≥5 `mcp__skillrank__*`
calls, that are *not* preceded by a user message mentioning skills. Baseline is 0 and 3
respectively. This is the minimum bar; failing it means the rewrite did nothing.

**A2 — It fires in the right band.** Agent-initiated invocations occur in **0.5%–1.5%**
of sessions in the window — the band occupied by `systematic-debugging` (0.52%),
`brainstorming` (0.66%), and `playwright-cli` (1.51%) on this machine. Below 0.5% the
rewrite has under-delivered; the response is R6, not a looser description.

**A3 — Nagware ceiling.** Agent-initiated invocations occur in **<3%** of sessions. At
or above 3%, revert to the user-only description and re-tune. This is a hard gate, and
it takes precedence over A1 and A2.

**A4 — Precision.** Of agent-initiated fires that led to an install, **≥50%** are still
present in `skill-registry-lock.json` 7 days later (checked via `skillrank list`, which
already reports drift and removal). An install the user rips out within a week is a
false positive that cost them more than a wasted search.

**A5 — No unrequested mutation.** Zero installs recorded on a turn where the transcript
contains no user confirmation between the suggestion and the install. Any occurrence is
a bug against R5, not a tuning issue.

**A6 — Off switch works.** After `skillrank setup --triggers=user-only`, a subsequent
`skillrank setup` and a subsequent `skillrank update` both leave the user-initiated
description in place. Verified by test, not by observation.

**A7 — Bootstrap works.** A machine with 0.1.4 installed, running `skillrank update` to
the version carrying this change, has the new description on disk afterwards, at both
the Claude and Codex paths, without running `setup` manually.

**A8 — User edits survive.** A machine whose `SKILL.md` has been hand-edited keeps that
edit through both `setup` and `update`, and is told once that a newer version exists.

**A9 — Default is non-invasive.** `skillrank setup` with no flags, and `install.sh`,
produce zero modifications to any file inside a user repository. Verified by test.

### The nagware tripwire

A3 and A4 together are the tripwire. If either trips, the correct response is to revert
the description, not to add a setting. A tool people mute is worse than a tool people
forget, because muting is not reversed.

Qualitatively, the signal to watch for is the user saying any variant of "stop looking
for skills" or "just fix it" in response to a suggestion. One such instance in the
window should be treated as equivalent to tripping A3.

## Edge Cases and Failure Modes

| Case | Required behavior |
| --- | --- |
| **Registry unreachable** on an agent-initiated fire | Fail silently back to the original task. Do not tell the user, do not retry. The user did not ask, so an error report is pure noise. (The existing "tell the user the registry isn't reachable" rule applies only to user-initiated use.) |
| **Fires inside an unrelated task** — user asked to fix a typo, agent hit a flaky test twice | The "next step is already clear" and "single failed command" gates should suppress it. If it fires anyway, R5 bounds the damage to one sentence and one search. |
| **Fires repeatedly in one long session** | "more than once per session" gate in the description. Advisory only — no enforcement mechanism exists without session state, which is a Non-Goal. Accepted risk; A3 detects it in aggregate. |
| **Both `skillrank` and `find-skills` fire** | Two overlapping skill-discovery skills already coexist in this surface (1 combined invocation across ~5,500 exposures). Duplicate suggestion is possible and low-harm. Out of scope to deduplicate; skillrank does not own the other skill. |
| **Nothing in the registry matches** | Say so in one sentence, return to the task. Never broaden the query, never suggest the closest-but-wrong skill. A weak suggestion is worse than none — it is the precise behavior that gets a tool muted. |
| **Matching skill has a non-`safe` scan tier** | Surface the tier in the same sentence as the suggestion. Never `--yes`. Existing install-time tier verification and rollback still apply. |
| **User is mid-incident / production debugging** | The gates cannot detect this. Accepted risk, bounded by R5 (read-only, non-blocking, one sentence). |
| **Repo has no `CLAUDE.md` and `--project` is used** | Refuse with a message. Do not create the file. |
| **Dirty working tree and `--project` is used** | Refuse. The user must be able to review the edit as an isolated diff. |
| **`--project` block already present** | Replace in place. Never append a second block. |
| **`SKILL.md` hand-edited by the user** | Never overwrite. Notice once, name the path, move on (R7). |
| **Skill directory deleted by the user** | Do not re-create on `setup` or `update` (R8). |
| **Codex vs Claude divergence** | Both get the same text via the same `SKILL_MD` constant. Codex's skill-selection behavior is not measured in this dataset; all rate targets in Acceptance Criteria are Claude Code figures, and Codex is unmeasured. Stated as an assumption, not a finding. |
| **CI / non-interactive** | `update_check` already skips when `CI` is set or stderr is not a TTY. The R7 notice inherits that. |

## Interfaces and Contracts

### Files whose content changes

| Path | Change |
| --- | --- |
| `crates/skillrank/src/skillrank_skill.md` | New frontmatter description (R1); body gains an agent-initiated triage section at the top (R3). |
| `crates/skillrank/src/mcp.rs` | `initialize.instructions`; `skill_search`, `skill_recommend`, `skill_install` descriptions (R2). |
| `crates/skillrank/src/skillrank_command.md` | Unchanged. |

### Files whose content is written to disk

| Path | Written by | Change |
| --- | --- | --- |
| `~/.claude/skills/skillrank/SKILL.md` | `setup`, and now `update` | New content; hash-guarded against user edits. |
| `~/.codex/skills/skillrank/SKILL.md` | `setup`, and now `update` | Same. |
| `~/.claude.json`, `~/.codex/config.toml` | `setup` | Unchanged behavior. |
| `~/.skillrank/` | `setup`, `update` | New: trigger-variant preference (R8) and one-shot notice marker (R7). |
| `<repo>/CLAUDE.md` or `AGENTS.md` | `setup --project` only | New, opt-in, marker-delimited, removable (R6). Never touched by default `setup` or `install.sh`. |

### CLI surface

| Command | Behavior |
| --- | --- |
| `skillrank setup` | Unchanged defaults, writes the new trigger. No repo writes. |
| `skillrank setup --triggers=user-only` | Writes the pre-rewrite description; persists the preference. |
| `skillrank setup --project` | Opt-in repo instruction block. |
| `skillrank setup --project --remove` | Removes the block by marker. |
| `skillrank setup --print` | Additionally reports which trigger variant would be written. |
| `skillrank update` | Additionally refreshes skill text, hash-guarded. |

### Behavioral contract for the agent

1. Agent-initiated entry ⇒ read-only commands only.
2. Install requires an explicit user "yes" on an agent-initiated path, regardless of
   scan tier.
3. At most one suggestion per session, one sentence, non-blocking.
4. No match, or registry unreachable ⇒ one sentence or silence, then back to the task.

Contracts 1, 2, and 4 are testable against transcripts. Contract 3 is advisory.

## Open Questions

1. **"Second attempt" vs "third attempt."** R1 proposes "second" on the reasoning that
   models under-count, so a third-attempt gate would likely never fire. This is a guess.
   It is the single highest-variance choice in the spec and the most likely cause of
   either A2 or A3 failing. No way to resolve it without shipping and measuring.
2. **Does the rewrite actually move the number?** The natural experiment is
   correlational. `systematic-debugging` may fire because debugging is common, not
   because its description is well-shaped. A1 is the falsification test.
3. **Is the description sufficient without R6?** Unresolved by design — R6 is staged
   behind the R1–R5 measurement precisely because the answer is unknown and the cost of
   guessing wrong (writing into someone else's repo) is high.
4. **Codex behavior is unmeasured.** All rates come from Claude Code transcripts. Codex
   may load skills differently or not at all. Whether Codex needs its own trigger
   variant is unknown.
5. **Attention displacement is not measured.** A skill description that fires more may
   suppress a *different* skill that would have been more useful. Nothing in the local
   transcripts distinguishes this from ordinary variance, and no cheap measurement
   exists.
6. **Single-machine dataset.** All 3,285 sessions are one user on one machine with an
   unusually large skill surface (51 global skills). Rates on a machine with 5 skills
   would likely be higher. The 0.5–1.5% band may not generalise.
7. **Should the ZeroShot side propose registry installs?** ZeroShot already proposes
   skills from real coding sessions, but only for authoring new ones. Extending it to
   recommend registry installs would catch exactly the cases the in-session trigger
   misses, after the fact, with better evidence and zero mid-task interruption cost.
   Deliberately out of scope; likely the stronger of the two halves and worth its own
   spec.
8. **Migrating `find-skills`.** The user's surface holds a second, redundant
   user-initiated skill-discovery skill. Whether skillrank should detect and mention it
   during `setup` is unresolved and arguably not skillrank's business.

## ExecPlan Handoff

No ExecPlan exists. An implementing ExecPlan must cover, in this order:

1. **R1 + R2 + R3** — text-only changes to `skillrank_skill.md` and `mcp.rs`. Zero
   behavior change in Rust logic. Ships first because it is the whole hypothesis and
   carries almost no implementation risk.
2. **R5** — the read-only / no-install-without-yes contract, expressed in the skill body
   and the `skill_install` tool description. Must land in the same release as (1); it is
   the guardrail that makes (1) safe to ship.
3. **R7 + R8** — `update` refreshes skill text with hash-guarded user-edit protection;
   `--triggers=user-only` preference persisted in `~/.skillrank/`; deleted skill
   directory is not re-created. Requires a shipped-version hash list and unit tests
   mirroring the existing `setup.rs` test style (temp-dir based, no network).
4. **Measurement harness** — a small local script that reproduces the Acceptance
   Criteria queries against `~/.claude/projects`. Must record the pre-change baseline
   (0 Skill invocations / 2,569 sessions; 3 MCP calls / 2,219) before the release lands,
   or A1–A4 are unmeasurable after the fact.
5. **R6** — held back. Do not implement until the (1)–(4) measurement window closes and
   shows a rate below the A2 band.

The ExecPlan must treat A3 (the <3% nagware ceiling) as a release gate with a named
revert commit prepared in advance, not as a metric to review later.
