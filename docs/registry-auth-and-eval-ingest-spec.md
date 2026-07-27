# Registry Accounts & Eval Ingest Spec

This spec defines desired behavior and acceptance criteria for the registry-side
work that the CLI and ZeroShot already assume exists. It is not an execution plan.

## Purpose

Make contributing back to SkillRank actually work, and make contributed evidence
trustworthy. Today the clients are written against a contribution API the
registry does not implement.

## Problem

The client contract exists; the server does not implement it.

- `skillrank-core::Client::submit_bundle` POSTs an `EvalBundle` to
  `/v3/rest/skill-registry/eval-results`. `registry/api/registry.mjs` handles
  `subscribe`, `installs`, `eval-suites/*`, and `skills/*`, and returns
  `404 {"error":"not found"}` for anything else. **Publishing an eval result
  cannot succeed.**
- `resolve_token()` attaches `Authorization: Bearer <token>` to write requests,
  and `skillrank login --token` stores one in `~/.skillrank/auth.json`. The
  registry never reads or verifies that header. **A token authenticates nothing.**
- The README advertises `publish` / `rate` / `review` as account-gated, and the
  skill pages present Official / Community-reported / Self-reported trust tiers
  whose meaning depends on attributable, independent submissions.

Consequences: ZeroShot ships a Publish button that 404s, `skillrank login` is
theatre, and the trust tiers have no mechanism behind them.

## Goals

- Publishing an eval bundle works end to end, from both the CLI and ZeroShot.
- Every published result is attributable to an account.
- Trust tiers reflect genuinely independent corroboration.
- ZeroShot users do not have to hand-copy a token.
- Reading stays fully anonymous — no account for search, install, or local eval.

## Non-Goals

- Human user accounts with passwords/profiles. Identity can be delegated
  (GitHub, BuildBetter/Keycloak); the registry needs a stable subject, not a
  login product.
- Paid tiers, quotas beyond abuse control, or org/team management.
- Retroactively attributing the existing seeded catalog.
- Moving the registry off its current static-JSON + KV architecture.

## Users and Actors

- **Anonymous reader** — CLI/app doing search, resolve, install, local eval.
- **Contributor** — an authenticated account publishing eval bundles, ratings,
  reviews, or indexing a public skill.
- **ZeroShot desktop** — already authenticated to BuildBetter; wants to
  provision a registry token without user copy-paste.
- **Registry maintainer** — needs revocation and abuse response.

## Requirements

### R1 — Token issuance

- An endpoint mints a token bound to a verified external identity (GitHub OAuth
  and/or BuildBetter/Keycloak). Anonymous self-service minting is **not**
  acceptable: it makes independence farmable (see R5).
- CLI path: device-code style — print a URL + code, user approves in browser,
  CLI polls and stores the token via the existing `save_token` path.
- ZeroShot path: exchange the existing BuildBetter session for a registry token
  with no user interaction, writing the same `~/.skillrank/auth.json` bytes so
  app and CLI share one identity.
- Tokens are opaque, revocable, and stored **hashed** server-side. The plaintext
  is shown/returned exactly once.

### R2 — Token verification

- Write endpoints reject a missing/invalid/revoked token with `401`.
- Read endpoints must remain unauthenticated and must not vary by token.
- Verification is constant-time against the stored hash.

### R3 — `POST /v3/rest/skill-registry/eval-results`

- Accepts the existing `EvalBundle` shape (`bundle_version`, `skill_slug`,
  `skill_content_hash`, `suite_id`, `suite_version`, `harness`,
  `environment_cell`, `trials[]`, `config_hash`, `created_at`).
- Responds with the existing `IngestResponse` (`accepted`, `result_id`,
  `tier_state`, `reason`, `conforming`) — the clients already parse this, so the
  contract is fixed by the wire types, not negotiable.
- Validation, each rejecting with a populated `reason`:
  - `skill_slug` exists; `skill_content_hash` matches a known published version
    (results must pin to reviewable content).
  - `suite_id` + `suite_version` exist; `config_hash` matches the suite's
    declared configuration (a "conforming" run).
  - Trial count matches the suite's task count × trials × 2 arms; arms balanced.
  - Bundle size and trial count bounded.
- Non-conforming bundles may be stored with `conforming: false` but must never
  be promoted above Self-reported.

### R4 — Idempotency and replay resistance

- `(account, skill_content_hash, suite_id, suite_version, config_hash, created_at)`
  identifies a submission; re-submitting returns the original `result_id` rather
  than inflating counts.
- Bundles older than a bounded window are rejected, so a single run cannot be
  re-published indefinitely to fake volume.

### R5 — Trust-tier integrity

- **Self-reported** — accepted from one account.
- **Community-reported** — requires corroboration from N independent accounts
  (N configurable, ≥3 recommended) on the same `(skill, content_hash, suite,
  environment cell)`, with results within an agreed variance band.
- **Official** — only from maintainer-run harnesses on the reference
  environment; never reachable by public submission.
- "Independent" must mean something: rate-limit per account, and require the
  identity provider to be one where creating N accounts has real cost. **If
  ZeroShot auto-provisions a token per install (R1), those accounts must not
  count toward Community independence** unless separately verified — otherwise
  N independent accounts collapses to N installs.
- Cells are never mixed across tiers (already the display contract).

### R6 — Abuse control

- Per-account and per-IP rate limits on writes; `429` with retry hint (the
  client already renders rate-limit errors).
- Revocation takes effect immediately; revoked-token submissions are rejected
  and previously published results are flagged for review, not silently kept.

## Acceptance Criteria

- `skillrank login` → `skillrank eval <ref> --suite <id> --publish` results in a
  stored, attributable result and a `200` with `accepted: true`.
- The same flow from ZeroShot's Publish button succeeds without the user ever
  seeing a token.
- Publishing without a token returns `401`; search/resolve/install still work
  with no token present.
- Re-publishing an identical bundle returns the original `result_id` and does
  not change any tier count.
- A skill with submissions from one account displays Self-reported; it only
  reaches Community-reported after N independent accounts corroborate.
- Revoking a token blocks subsequent publishes immediately.

## Edge Cases and Failure Modes

- Registry unreachable mid-publish: client already surfaces the error; ingest
  must be idempotent so a retry is safe.
- Clock skew on `created_at` — validate against a tolerant window.
- A skill re-tiered or tombstoned after a result was published: results stay
  attached to the `content_hash` they measured, and the UI must not present them
  as describing current content.
- Token present but for a deleted account → `401`, same as invalid.

## Interfaces and Contracts

- Wire types are already fixed by `skillrank-core::types` (`EvalBundle`,
  `IngestResponse`) — the server must conform to them; changing them is a
  breaking change for every installed CLI.
- Token transport: `Authorization: Bearer <token>` (already implemented
  client-side).
- Token storage: `~/.skillrank/auth.json`, `{"token": "..."}` — shared by the
  CLI and ZeroShot.

## Open Questions

- Which identity provider(s) for the CLI path — GitHub OAuth is the obvious fit
  for a developer registry; BuildBetter/Keycloak covers ZeroShot for free.
- Does auto-provisioned ZeroShot identity ever earn Community independence, and
  under what additional verification?
- Where do accounts and results live? Current stack is static JSON + Upstash KV;
  hashed tokens and result rows likely want a real datastore.
- Should `rate` / `review` land in the same milestone as eval ingest, or after?

## ExecPlan Handoff

Sequencing: R3 (ingest endpoint, tokenless behind a flag) → R1/R2 (issuance +
verification) → R5 (tier promotion) → R6 (limits/revocation). ZeroShot's
auto-provisioning and Settings key view depend on R1/R2 and should not start
before the token contract is real.

Until R3 ships, ZeroShot's Publish button 404s; consider hiding or disabling it
in the app rather than presenting an action that cannot succeed.
