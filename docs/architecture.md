# SkillRank architecture

## The pieces and who depends on whom

```
                         ┌──────────────────────────┐
   this repo (OSS) ───►  │  skillrank CLI + runner  │  standalone binary
                         │  (Go, MIT, no BuildBetter │  `curl .../install.sh | sh`
                         │   dependency at all)      │
                         └────────────┬─────────────┘
                                      │ talks over HTTP to
                                      ▼
                         ┌──────────────────────────┐
   hosted service ─────► │   SkillRank registry API │  search, resolve, publish,
                         │   (search/publish/evals/  │  reviews, official baselines
                         │    reviews/leaderboards)  │  SKILLRANK_API_URL points here
                         └──────────────────────────┘

                         ┌──────────────────────────┐
   BuildBetter ZeroShot  │  ZeroShot desktop app +   │  EMBEDS this module to provide
   (separate product) ─► │  `bb` CLI                 │  `bb skills …`; recommends skills
                         │                           │  from real sessions; tracks
                         │                           │  realized savings
                         └──────────────────────────┘
```

**Dependency direction (the important part):** SkillRank depends on nothing.
ZeroShot depends on SkillRank — it embeds this module so `bb skills <cmd>` is the
same code as `skillrank <cmd>`, and its installer/onboarding can offer SkillRank.
SkillRank never imports ZeroShot or any BuildBetter package. A user can install
`skillrank` alone and use every core feature; ZeroShot is a strictly optional
enhancement.

**The installer inverts the bundling, too:** `install.sh` installs `skillrank`
and then *offers* to also install ZeroShot. ZeroShot's own installer, in turn,
includes SkillRank. Either entry point works; neither is required by the other.

## Why the CLI has no external Go dependencies

The binary is stdlib-only (`internal/{command,config,api,skills}` are ~small,
self-contained). That keeps it trivial to build, audit, and vendor, and means the
open-source runner that produces published eval numbers can be inspected by
anyone — which is the whole credibility argument for the benchmark.

## The eval harness is shared

`internal/registry/runner` is the single eval harness. The hosted service runs
**the same code** for official baselines and audit re-runs that users run for
community evals — so official and community numbers are directly comparable, and
every published number traces back to auditable OSS code. Non-Docker runs and
runs off the pinned reference agent version are labeled Self-reported and never
aggregated into Community-reported.

## Where the registry backend lives

The hosted registry API (the service `SKILLRANK_API_URL` points at) is a separate
codebase. BuildBetter's reference implementation is a domain package + REST
controllers + an SQS worker (community catalog, trust-tier state machine,
security scanning, score aggregation). That backend can itself be open-sourced
later; the CLI only depends on its HTTP contract, not its code. The wire contract
is defined by the Go types in `internal/registry/types.go` and the eval-bundle
JSON schema the runner emits.

## Relationship to the earlier `bb skills` build

An initial embedded copy of this CLI was built directly inside BuildBetter's
private monorepo as `bb skills`. That was the wrong dependency direction. This
repo is now the source of truth; ZeroShot/`bb` should import this module (once
published) or bundle the released binary, and the embedded copy retired. The
hosted registry backend built in the monorepo stays as the service this CLI
talks to.

## Configuration surface

| Env | Purpose | Default |
|---|---|---|
| `SKILLRANK_API_URL` | registry base URL | `https://api.skillrank.dev` |
| `SKILLRANK_TOKEN` | token for writes (publish/rate/review) | unset (reads are anonymous) |
| `SKILLRANK_HOME` | config/auth dir | `~/.skillrank` |
