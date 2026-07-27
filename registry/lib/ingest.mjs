// Eval-result ingest: validation, idempotency, and trust-tier computation
// (spec R3/R4/R5).
//
// Three properties drive the design:
//
//  1. Nothing here takes a DERIVED value on trust. `config_hash` and `conforming`
//     are recomputed server-side rather than read off the wire, so a bundle
//     cannot disagree with itself. What they are recomputed FROM is still the
//     client's own description of its run: `environment_cell` (and therefore
//     `conforming`) is self-attested, exactly like the trial verdicts next to it.
//     Making it verifiable needs a signed harness attestation, which the wire
//     types have no field for — see `isConforming`.
//  2. Replay cannot inflate anything, structurally. The `result_id` is a pure
//     function of the spec's idempotency tuple, and every aggregate is a set or a
//     hash keyed by that id — so re-submitting the same bundle rewrites the same
//     fields instead of appending. That also makes a partially-failed write safe
//     to retry: the result record is written last, so a retry re-runs the whole
//     sequence and converges. `created_at` is client-chosen and is the tuple's
//     only free variable, so per-account admission control (not the tuple alone)
//     is what bounds how much of a cell one account can occupy.
//  3. A badge and the numbers printed beside it describe the same population.
//     Community-reported is earned by verified accounts, so a corroborated cell
//     reports the verified population's numbers; anonymous contributions are
//     counted separately rather than folded silently into the headline.
//
// `ingestBundle` returns the exact `{ status, body }` the route emits. The HTTP
// shape is the contract (`skillrank-core::types::IngestResponse`) and keeping it
// here means it is unit-testable without booting a server.

import { createHash } from "node:crypto";

import { KEYS } from "./store.mjs";

/// The only harness whose numbers are comparable. Bundles must come from it.
export const HARNESS_NAME = "skillrank-runner";

export const TIER_SELF = "self_reported";
export const TIER_COMMUNITY = "community_reported";
/// Never produced by this module: Official is maintainer-run only (R5).
export const TIER_OFFICIAL = "official";

const HASH_PREFIX = "sha256:";
const HASH_RE = /^(sha256:)?[0-9a-f]{64}$/i;
// Every segment must start alphanumeric, which rules out "." and ".." segments.
// The slug becomes Redis key material and is echoed back in read responses, so it
// is validated for shape here and for existence against the catalog below —
// never reflected verbatim. All 2079 catalog slugs satisfy this.
const SLUG_RE = /^[A-Za-z0-9][A-Za-z0-9._-]*(?:\/[A-Za-z0-9][A-Za-z0-9._-]*)*$/;
const CELL_VALUE_RE = /^[A-Za-z0-9._:+-]*$/;
// The harness version is a comparability determinant (the Rust runner bakes it
// into `config_hash`) and it is now part of the cell id, so it has to be a
// bounded, meaningful value rather than free-form text — otherwise every junk
// string mints its own cell. `skillrank-runner` versions are CARGO_PKG_VERSION.
const HARNESS_VERSION_RE = /^\d{1,5}\.\d{1,5}\.\d{1,5}(?:-[A-Za-z0-9.]{1,16})?$/;
const TIMESTAMP_RE = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/;
const VERDICTS = new Set(["pass", "fail", "agent_error", "verifier_error"]);
const ARMS = new Set(["control", "treatment"]);
const ISOLATIONS = new Set(["docker", "worktree"]);
const COUNTER_MAX = 1e12;

function intEnv(raw, fallback) {
  const n = Number.parseInt(String(raw ?? ""), 10);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

function floatEnv(raw, fallback) {
  const n = Number.parseFloat(String(raw ?? ""));
  return Number.isFinite(n) && n >= 0 ? n : fallback;
}

/// All ingest limits, read per request so a Vercel env change takes effect
/// without a redeploy.
export function ingestConfig(env = {}) {
  const maxResultsPerCell = intEnv(env.EVAL_MAX_RESULTS_PER_CELL, 500);
  return {
    maxBodyBytes: intEnv(env.EVAL_MAX_BODY_BYTES, 256 * 1024),
    maxTrialsPerArm: intEnv(env.EVAL_MAX_TRIALS_PER_ARM, 25),
    // Generous on purpose: ZeroShot republishes bundles listed from
    // ~/.skillrank/bundles, which are routinely days or weeks old.
    maxAgeDays: intEnv(env.EVAL_MAX_AGE_DAYS, 30),
    maxFutureSkewHours: intEnv(env.EVAL_MAX_FUTURE_SKEW_HOURS, 24),
    // Floored at 2: with N=1 "Community-reported" would mean exactly what
    // "Self-reported" already means, and the published tier definitions would
    // stop being true. The spec recommends 3.
    minAccounts: Math.max(2, intEnv(env.COMMUNITY_MIN_ACCOUNTS, 3)),
    varianceBand: floatEnv(env.COMMUNITY_VARIANCE_BAND, 0.25),
    maxResultsPerCell,
    // Per-account share of a cell. `created_at` is client-chosen, so the R4 tuple
    // alone lets one account register the same physical run thousands of times;
    // this is what actually bounds it. Kept well under `maxResultsPerCell` so
    // filling a cell takes many accounts, not one.
    maxResultsPerAccountPerCell: Math.min(
      maxResultsPerCell,
      Math.max(1, intEnv(env.EVAL_MAX_RESULTS_PER_ACCOUNT_PER_CELL, 10)),
    ),
    // `environment_cell` is free-form client text, so cell identity is
    // attacker-chosen and the public read fans out over every cell of a slug in
    // one unpaginated response. Both a global ceiling and a per-account
    // creation budget are needed: the ceiling alone would be a lockout lever.
    maxCellsPerSkill: intEnv(env.EVAL_MAX_CELLS_PER_SKILL, 100),
    newCellsPerAccountPerDay: intEnv(env.EVAL_NEW_CELLS_PER_ACCOUNT_PER_DAY, 5),
    // Every `eval:*` key is written with this lifetime and re-armed on each
    // touch, so an abandoned cell ages out instead of pinning storage forever.
    // Far longer than `maxAgeDays`, so it never rejects a publishable bundle.
    retentionSeconds: intEnv(env.EVAL_RETENTION_DAYS, 400) * 86_400,
  };
}

export function sha256Hex(value) {
  return createHash("sha256").update(value, "utf8").digest("hex");
}

export function normalizeHash(raw) {
  return HASH_PREFIX + String(raw).replace(new RegExp(`^${HASH_PREFIX}`, "i"), "").toLowerCase();
}

/// Mirrors `skillrank_core::hash::hashes_equal`: prefix-tolerant on either side,
/// ASCII case-insensitive.
export function hashesEqual(a, b) {
  if (typeof a !== "string" || typeof b !== "string") return false;
  return normalizeHash(a) === normalizeHash(b);
}

/// Byte-for-byte reimplementation of `skillrank_core::runner::compute_config_hash`.
/// `trialsPerArm` is not on the wire; it is derived from the trial count.
export function computeConfigHash({ harness, suite, slug, contentHash, trialsPerArm, cell }) {
  const canonical =
    `harness=${harness.name}/${harness.version}` +
    `|suite=${suite.id}@${suite.version}` +
    `|skill=${slug}@${contentHash}` +
    `|trials=${trialsPerArm}` +
    `|agent=${cell.agent}|band=${cell.agent_version_band}|model=${cell.model}` +
    `|os=${cell.os}|isolation=${cell.isolation}`;
  return HASH_PREFIX + sha256Hex(canonical);
}

/// Reimplementation of `skillrank_core::runner::is_conforming`. This is a
/// property of the (suite, environment cell) pair alone, so every result in a
/// given cell shares it — tiers can never get mixed inside one cell.
///
/// SELF-ATTESTED, and deliberately documented as such. Locally the runner
/// OBSERVES the cell (`runner::fixtures::isolation`); on the wire the cell is
/// whatever the publisher says it is, and recomputing `config_hash` over those
/// same fields proves self-consistency, not provenance. Relabelling a worktree
/// run as `docker` is one sha256 away. That is not the marginal weakness — the
/// trial verdicts are equally unattested, and R5 puts the anti-gaming burden on
/// the cost of verified accounts — but it does mean "conforming" must be read as
/// "claims a conforming environment". Making it verifiable needs a signed
/// harness attestation over the observed cell, which `EvalBundle` has no field
/// for; adding one is a wire-contract change and therefore future work.
export function isConforming(referenceEnv, cell) {
  if (cell.isolation !== "docker") return false;
  const band = referenceEnv?.agent_version_band || "";
  if (band && band !== cell.agent_version_band) return false;
  const models = Array.isArray(referenceEnv?.models) ? referenceEnv.models : [];
  if (models.length > 0 && !models.includes(cell.model)) return false;
  return true;
}

/// Stable identity for a comparable (skill, content_hash, suite, harness,
/// environment) cell.
///
/// The harness is part of the identity because it is part of comparability: the
/// Rust runner bakes its version into `config_hash` for exactly that reason, and
/// leaving it out let results from mutually incomparable runner versions pool
/// into one cell and corroborate each other up to Community-reported. The cost
/// is that a runner release starts fresh cells rather than inheriting the old
/// ones — which is the honest outcome, since the numbers are not comparable.
///
/// The `cell_` prefix is load-bearing: these ids are stored as Redis set members,
/// and the Upstash client JSON-parses responses, so an all-digit id would come
/// back as a lossy Number.
export function cellIdFor({ slug, contentHash, suiteId, suiteVersion, harness, cell }) {
  const canonical = [
    slug,
    normalizeHash(contentHash),
    `${suiteId}@${suiteVersion}`,
    `${harness.name}/${harness.version}`,
    cell.agent,
    cell.agent_version_band,
    cell.model,
    cell.os,
    cell.isolation,
  ].join("|");
  return `cell_${sha256Hex(canonical)}`;
}

/// The spec's R4 tuple. Deliberately excludes `skill_slug` (the content hash
/// already pins the content) and includes the account, so two people running the
/// same config are two submissions rather than one.
export function idempotencyKey({ accountId, contentHash, suiteId, suiteVersion, configHash, createdAt }) {
  return sha256Hex(
    ["v1", accountId, normalizeHash(contentHash), suiteId, suiteVersion, configHash, createdAt].join("|"),
  );
}

export function resultIdFor(idemKey) {
  return `res_${idemKey.slice(0, 32)}`;
}

function isStr(value, max) {
  return typeof value === "string" && value.length <= max;
}

function isCount(value) {
  return Number.isSafeInteger(value) && value >= 0 && value <= COUNTER_MAX;
}

function fail(reason) {
  return { ok: false, reason };
}

/// Per-arm pass/token totals. Tokens are input+output only, matching what the
/// local runner reports, so published deltas equal what the user saw.
export function summarizeTrials(trials) {
  const acc = {
    control_pass: 0,
    control_total: 0,
    control_tokens: 0,
    treatment_pass: 0,
    treatment_total: 0,
    treatment_tokens: 0,
  };
  for (const t of trials) {
    const arm = t.arm === "treatment" ? "treatment" : "control";
    acc[`${arm}_total`] += 1;
    acc[`${arm}_tokens`] += (t.input_tokens || 0) + (t.output_tokens || 0);
    if (t.verdict === "pass") acc[`${arm}_pass`] += 1;
  }
  return acc;
}

/// Everything checkable without touching the store or the catalog.
export function validateBundleShape(bundle, { suites, config, now }) {
  if (!bundle || typeof bundle !== "object" || Array.isArray(bundle)) return fail("body must be a JSON object");
  if (bundle.bundle_version !== 1) return fail("unsupported bundle_version; this registry accepts version 1");

  if (!isStr(bundle.skill_slug, 200) || !SLUG_RE.test(bundle.skill_slug)) return fail("skill_slug is malformed");
  if (!isStr(bundle.skill_content_hash, 100) || !HASH_RE.test(bundle.skill_content_hash)) {
    return fail("skill_content_hash must be sha256:<64 hex>");
  }
  if (!isStr(bundle.suite_id, 100) || !bundle.suite_id) return fail("suite_id is required");
  if (!isStr(bundle.suite_version, 32) || !bundle.suite_version) return fail("suite_version is required");

  const suite = (suites || []).find((s) => s.id === bundle.suite_id);
  if (!suite) return fail(`unknown suite "${bundle.suite_id}"`);
  if (suite.version !== bundle.suite_version) {
    return fail(`suite ${suite.id} is at version ${suite.version}, not ${bundle.suite_version}`);
  }
  const tasks = Array.isArray(suite.tasks) ? suite.tasks : [];
  if (tasks.length === 0) return fail(`suite ${suite.id} declares no tasks`);

  const harness = bundle.harness;
  if (!harness || typeof harness !== "object" || !isStr(harness.name, 64) || !isStr(harness.version, 32)) {
    return fail("harness must be { name, version }");
  }
  if (harness.name !== HARNESS_NAME) {
    return fail(`results must come from the ${HARNESS_NAME} harness so they stay comparable`);
  }
  if (!harness.version) return fail("harness.version is required");
  if (!HARNESS_VERSION_RE.test(harness.version)) {
    return fail(`harness.version must be a ${HARNESS_NAME} release version like 0.1.0`);
  }

  const cell = bundle.environment_cell;
  if (!cell || typeof cell !== "object" || Array.isArray(cell)) return fail("environment_cell is required");
  for (const field of ["agent", "agent_version_band", "model", "os", "isolation"]) {
    if (!isStr(cell[field], 64) || !CELL_VALUE_RE.test(cell[field])) {
      return fail(`environment_cell.${field} is malformed`);
    }
  }
  if (!cell.agent) return fail("environment_cell.agent is required");
  if (!cell.os) return fail("environment_cell.os is required");
  if (!ISOLATIONS.has(cell.isolation)) return fail('environment_cell.isolation must be "docker" or "worktree"');

  if (!isStr(bundle.created_at, 40) || !TIMESTAMP_RE.test(bundle.created_at)) {
    return fail("created_at must be an RFC 3339 timestamp");
  }
  const createdMs = Date.parse(bundle.created_at);
  if (!Number.isFinite(createdMs)) return fail("created_at is not a valid timestamp");
  const ageDays = (now - createdMs) / 86_400_000;
  if (ageDays > config.maxAgeDays) {
    return fail(`this run is older than ${config.maxAgeDays} days; re-run the eval before publishing`);
  }
  if (createdMs - now > config.maxFutureSkewHours * 3_600_000) {
    return fail("created_at is in the future; check this machine's clock");
  }

  const trials = bundle.trials;
  if (!Array.isArray(trials) || trials.length === 0) return fail("trials must be a non-empty array");
  const expectedGroups = tasks.length * 2;
  if (trials.length % expectedGroups !== 0) {
    return fail(`suite ${suite.id} has ${tasks.length} task(s), so trials must be a multiple of ${expectedGroups}`);
  }
  const trialsPerArm = trials.length / expectedGroups;
  if (trialsPerArm > config.maxTrialsPerArm) {
    return fail(`at most ${config.maxTrialsPerArm} trials per arm may be published`);
  }

  const taskIds = new Set(tasks.map((t) => t.id));
  const groups = new Map();
  for (const trial of trials) {
    if (!trial || typeof trial !== "object" || Array.isArray(trial)) return fail("each trial must be an object");
    if (!isStr(trial.task_id, 100) || !taskIds.has(trial.task_id)) {
      return fail(`trial task_id "${String(trial.task_id).slice(0, 40)}" is not in suite ${suite.id}`);
    }
    if (!isStr(trial.arm, 16) || !ARMS.has(trial.arm)) return fail('trial arm must be "control" or "treatment"');
    if (!isStr(trial.verdict, 32) || !VERDICTS.has(trial.verdict)) return fail("trial verdict is not recognized");
    for (const field of ["input_tokens", "output_tokens", "cache_read_tokens", "cache_write_tokens", "duration_ms", "turns"]) {
      if (trial[field] !== undefined && !isCount(trial[field])) return fail(`trial ${field} must be a non-negative integer`);
    }
    if (trial.cost_usd !== undefined && trial.cost_usd !== null) {
      if (typeof trial.cost_usd !== "number" || !Number.isFinite(trial.cost_usd) || trial.cost_usd < 0) {
        return fail("trial cost_usd must be a non-negative number");
      }
    }
    if (trial.trajectory_digest !== undefined && trial.trajectory_digest !== "") {
      if (!isStr(trial.trajectory_digest, 100) || !HASH_RE.test(trial.trajectory_digest)) {
        return fail("trial trajectory_digest must be sha256:<64 hex>");
      }
    }
    const key = `${trial.task_id} ${trial.arm}`;
    groups.set(key, (groups.get(key) || 0) + 1);
  }
  if (groups.size !== expectedGroups) return fail("every task must have both a control and a treatment arm");
  for (const count of groups.values()) {
    if (count !== trialsPerArm) return fail("arms are unbalanced; every task/arm pair needs the same trial count");
  }

  if (!isStr(bundle.config_hash, 100) || !HASH_RE.test(bundle.config_hash)) {
    return fail("config_hash must be sha256:<64 hex>");
  }
  const expected = computeConfigHash({
    harness,
    suite,
    slug: bundle.skill_slug,
    contentHash: bundle.skill_content_hash,
    trialsPerArm,
    cell,
  });
  if (!hashesEqual(expected, bundle.config_hash)) {
    return fail(`config_hash does not describe this run of ${suite.id}@${suite.version}`);
  }

  const projectedCell = {
    agent: cell.agent,
    agent_version_band: cell.agent_version_band,
    model: cell.model,
    os: cell.os,
    isolation: cell.isolation,
  };
  const projectedHarness = { name: harness.name, version: harness.version };
  const contentHash = normalizeHash(bundle.skill_content_hash);
  const configHash = normalizeHash(bundle.config_hash);

  return {
    ok: true,
    reason: "",
    value: {
      suite,
      cell: projectedCell,
      harness: projectedHarness,
      trialsPerArm,
      totals: summarizeTrials(trials),
      conforming: isConforming(suite.reference_env, cell),
      contentHash,
      configHash,
      // What gets PERSISTED as the raw evidence. Never the request object: nothing
      // rejects unknown top-level keys, so storing the client's own object pins
      // arbitrary attacker-supplied padding (up to the whole body limit) in a
      // TTL-less key. This projection holds every field the bundle contract
      // defines and nothing else, so its size is bounded by the trial cap.
      normalized: {
        bundle_version: 1,
        skill_slug: bundle.skill_slug,
        skill_content_hash: contentHash,
        suite_id: suite.id,
        suite_version: suite.version,
        harness: projectedHarness,
        environment_cell: projectedCell,
        trials: trials.map(normalizeTrial),
        config_hash: configHash,
        created_at: bundle.created_at,
      },
    },
  };
}

/// Project one trial onto the `TrialRecord` contract, dropping anything else.
function normalizeTrial(trial) {
  const out = {
    task_id: trial.task_id,
    arm: trial.arm,
    verdict: trial.verdict,
    input_tokens: trial.input_tokens || 0,
    output_tokens: trial.output_tokens || 0,
    cache_read_tokens: trial.cache_read_tokens || 0,
    cache_write_tokens: trial.cache_write_tokens || 0,
    duration_ms: trial.duration_ms || 0,
    turns: trial.turns || 0,
  };
  // Both are `skip_serializing_if` on the Rust side, so absent stays absent.
  if (typeof trial.cost_usd === "number") out.cost_usd = trial.cost_usd;
  if (trial.trajectory_digest) out.trajectory_digest = trial.trajectory_digest;
  return out;
}

function round(value, digits) {
  const f = 10 ** digits;
  return Math.round(value * f) / f;
}

const TOTAL_KEYS = [
  "control_pass",
  "control_total",
  "control_tokens",
  "treatment_pass",
  "treatment_total",
  "treatment_tokens",
];

/// Pool each account's results into one row. Everything downstream is computed
/// per account rather than per result, so an account that published a run fifty
/// times carries exactly the weight of one that published it once.
function foldByAccount(summaries) {
  const byAccount = new Map();
  for (const s of summaries) {
    let row = byAccount.get(s.account_id);
    if (!row) {
      row = { account_id: s.account_id, verified: false, n_results: 0 };
      for (const key of TOTAL_KEYS) row[key] = 0;
      byAccount.set(s.account_id, row);
    }
    row.verified = row.verified || s.verified === true;
    row.n_results += 1;
    for (const key of TOTAL_KEYS) row[key] += s[key] || 0;
  }
  return [...byAccount.values()];
}

const rate = (pass, total) => (total ? pass / total : 0);

function accountDelta(row) {
  return rate(row.treatment_pass, row.treatment_total) - rate(row.control_pass, row.control_total);
}

/// Mean of the per-account rates over one population, plus that population's
/// trial count. Unweighted on purpose: pooling raw totals would let whoever
/// published the most trials set the headline number.
function meanRates(rows) {
  const out = { control: 0, treatment: 0, controlTokens: 0, treatmentTokens: 0, trials: 0, results: 0 };
  if (rows.length === 0) return out;
  for (const row of rows) {
    out.control += rate(row.control_pass, row.control_total);
    out.treatment += rate(row.treatment_pass, row.treatment_total);
    out.controlTokens += rate(row.control_tokens, row.control_total);
    out.treatmentTokens += rate(row.treatment_tokens, row.treatment_total);
    out.trials += row.control_total + row.treatment_total;
    out.results += row.n_results;
  }
  for (const key of ["control", "treatment", "controlTokens", "treatmentTokens"]) out[key] /= rows.length;
  return out;
}

/// Roll a cell's per-result summaries into the numbers a skill page shows, and
/// decide its trust tier.
///
/// Community requires: a conforming cell, N distinct VERIFIED accounts, and their
/// per-account pass-rate deltas agreeing within the variance band.
///
/// The population that earns the badge is also the population the displayed
/// numbers describe (`metrics_basis`). Folding anonymous results into the rates
/// under a Community-reported badge would let one free auto-provisioned token
/// rewrite corroborated evidence it was explicitly excluded from earning, so a
/// corroborated cell reports the verified accounts' numbers and reports the
/// anonymous contribution beside them as its own count. A self-reported cell has
/// no such split — "self-reported" already means "whoever reported it" — so its
/// numbers cover every account.
export function computeCellAggregate(summaries, { minAccounts, varianceBand, conforming }) {
  const live = summaries.filter((s) => !s.flagged);
  const rows = foldByAccount(live);
  const verified = rows.filter((r) => r.verified);
  const anonymous = rows.filter((r) => !r.verified);

  const deltas = verified.map(accountDelta);
  const spread = deltas.length > 1 ? Math.max(...deltas) - Math.min(...deltas) : 0;
  const corroborated = Boolean(conforming) && verified.length >= minAccounts && spread <= varianceBand;

  const basis = corroborated ? verified : rows;
  const stats = meanRates(basis);

  return {
    tier_state: corroborated ? TIER_COMMUNITY : TIER_SELF,
    conforming: Boolean(conforming),
    metrics_basis: corroborated ? "verified_accounts" : "all_accounts",
    n_results: stats.results,
    n_accounts: basis.length,
    n_verified_accounts: verified.length,
    n_anonymous_accounts: anonymous.length,
    n_anonymous_results: anonymous.reduce((sum, r) => sum + r.n_results, 0),
    n_flagged_results: summaries.length - live.length,
    n_trials: stats.trials,
    control_pass_rate: round(stats.control, 4),
    treatment_pass_rate: round(stats.treatment, 4),
    success_delta_pct: round((stats.treatment - stats.control) * 100, 1),
    token_delta_pct:
      stats.controlTokens > 0 ? round(((stats.treatmentTokens - stats.controlTokens) / stats.controlTokens) * 100, 1) : 0,
    variance_spread: round(spread, 4),
  };
}

function reject(reason) {
  return { status: 400, body: { accepted: false, error: reason, reason } };
}

const olderFirst = (a, b) =>
  String(a.value.received_at || "").localeCompare(String(b.value.received_at || ""));

/// Order a cell's retained summaries worst-to-keep first.
///
/// Flagged entries go first: a revoked account's results are dead weight, which
/// is what makes R6 revocation actually free capacity. Among live entries the
/// victim is the account holding the most slots (anonymous before verified on a
/// tie, oldest first within the account). Evicting the biggest holder is what
/// makes flooding self-limiting — a spammer's own surplus is displaced before a
/// genuine contributor's single result, so filling a cell can never lock honest
/// corroboration out of it.
function evictionOrder(entries) {
  const holdings = new Map();
  for (const e of entries) {
    if (e.value.flagged) continue;
    holdings.set(e.value.account_id, (holdings.get(e.value.account_id) || 0) + 1);
  }
  return [...entries].sort((a, b) => {
    const flaggedRank = (a.value.flagged ? 0 : 1) - (b.value.flagged ? 0 : 1);
    if (flaggedRank !== 0) return flaggedRank;
    const held = (holdings.get(b.value.account_id) || 0) - (holdings.get(a.value.account_id) || 0);
    if (held !== 0) return held;
    const verifiedRank = (a.value.verified ? 1 : 0) - (b.value.verified ? 1 : 0);
    if (verifiedRank !== 0) return verifiedRank;
    return olderFirst(a, b);
  });
}

/// Free a slot for the submission in hand, evicting rather than dropping.
///
/// The incoming result is ALWAYS stored: answering `accepted: true` for a result
/// we silently discarded is a lie, and a cell that swallows honest submissions
/// once it is full can never gain the corroboration that would move it off
/// Self-reported — no route can delete summaries, so that state was permanent.
///
/// Two ceilings apply. An account at its per-cell share replaces its OWN oldest
/// retained result, so re-registering one run under fresh `created_at` values
/// converges on a bounded window instead of accumulating. Then the cell's own
/// ceiling applies, evicting by `evictionOrder`.
///
/// Returns the evicted field names (already removed from the hash).
async function admitSummary(store, key, entries, { accountId, incomingField, maxPerCell, maxPerAccount }) {
  const alive = new Map(entries.filter((e) => e.field !== incomingField).map((e) => [e.field, e]));
  const evicted = [];
  const drop = (entry) => {
    alive.delete(entry.field);
    evicted.push(entry.field);
  };

  const ownLive = () =>
    [...alive.values()].filter((e) => !e.value.flagged && e.value.account_id === accountId).sort(olderFirst);
  for (let own = ownLive(); own.length >= maxPerAccount; own = ownLive()) drop(own[0]);

  while (alive.size >= maxPerCell) {
    const victim = evictionOrder([...alive.values()])[0];
    if (!victim) break;
    drop(victim);
  }

  for (const field of evicted) await store.hdel(key, field);
  return new Set(evicted);
}

/// Validate, deduplicate, store, and tier one bundle. Throws only if the store
/// throws — the caller turns that into a loud 503 rather than a silent success.
export async function ingestBundle({ store, bundle, account, suites, catalog, config, now }) {
  const shape = validateBundleShape(bundle, { suites, config, now: now.getTime() });
  if (!shape.ok) return reject(shape.reason);
  const { suite, cell, harness, trialsPerArm, totals, conforming, contentHash, configHash, normalized } = shape.value;
  const slug = bundle.skill_slug;

  if (!catalog.hasSkill(slug)) return reject(`unknown skill "${slug}"`);

  // R3: results must pin to reviewable content. The catalog holds one hash per
  // skill, so we also accept any hash this registry has already published results
  // against — otherwise a catalog refresh would retroactively invalidate every
  // in-flight bundle, and republishing an older run would become impossible.
  const publishedHash = catalog.publishedHash(slug);
  const knownHashes = await store.smembers(KEYS.skillHashes(slug));
  const known = [publishedHash, ...knownHashes].some((h) => h && hashesEqual(h, contentHash));
  if (!known) {
    return reject(`skill_content_hash does not match a published version of ${slug}; re-resolve the skill and re-run`);
  }

  const cellId = cellIdFor({ slug, contentHash, suiteId: suite.id, suiteVersion: suite.version, harness, cell });
  const idemKey = idempotencyKey({
    accountId: account.account_id,
    contentHash,
    suiteId: suite.id,
    suiteVersion: suite.version,
    configHash,
    createdAt: bundle.created_at,
  });
  const resultId = resultIdFor(idemKey);

  const existing = await store.getJson(KEYS.result(resultId));
  if (existing) {
    // R4: the original id, and no count moves. The tier is re-read rather than
    // replayed, so a retry reflects corroboration that landed in the meantime.
    const meta = await store.getJson(KEYS.cell(cellId));
    return {
      status: 200,
      body: {
        accepted: true,
        result_id: resultId,
        tier_state: (meta && meta.tier_state) || existing.tier_state || TIER_SELF,
        conforming: Boolean(existing.conforming),
        reason: "already published; returning the original result",
        duplicate: true,
      },
    };
  }

  // Cell cardinality. Every field `cellIdFor` hashes is client-supplied text, so
  // without a ceiling one account mints unlimited fabricated cells on any slug —
  // and the unauthenticated read below returns all of them in one unpaginated
  // response. The per-account daily budget exists so the ceiling cannot be turned
  // into a lockout: filling a slug's cell list takes many accounts and many days.
  const knownCells = await store.smembers(KEYS.skillCells(slug));
  if (!knownCells.includes(cellId)) {
    if (knownCells.length >= config.maxCellsPerSkill) {
      return reject(
        `${slug} already has ${config.maxCellsPerSkill} environment cells; publish into one that already exists`,
      );
    }
    const budget = await store.hit(
      `newcell:${slug}:${account.account_id}`,
      config.newCellsPerAccountPerDay,
      86_400,
    );
    if (!budget.allowed) {
      return reject(
        `this account has opened ${config.newCellsPerAccountPerDay} new environment cells for ${slug} today; retry in ${budget.retryAfter} seconds`,
      );
    }
  }

  const verified = account.verified === true;
  const summary = {
    account_id: account.account_id,
    verified,
    conforming,
    created_at: bundle.created_at,
    received_at: now.toISOString(),
    trials_per_arm: trialsPerArm,
    flagged: false,
    ...totals,
  };

  const ttl = config.retentionSeconds;
  await store.sadd(KEYS.skillHashes(slug), contentHash);
  await store.sadd(KEYS.skillCells(slug), cellId);
  await store.sadd(KEYS.accountCells(account.account_id), cellId);
  // Re-armed on every publish, so an active skill never expires and an abandoned
  // one stops costing storage.
  await store.expire(KEYS.skillHashes(slug), ttl);
  await store.expire(KEYS.skillCells(slug), ttl);
  await store.expire(KEYS.accountCells(account.account_id), ttl);

  const cellSummariesKey = KEYS.cellSummaries(cellId);
  const entries = await store.hgetallJson(cellSummariesKey);
  const evicted = await admitSummary(store, cellSummariesKey, entries, {
    accountId: account.account_id,
    incomingField: resultId,
    maxPerCell: config.maxResultsPerCell,
    maxPerAccount: config.maxResultsPerAccountPerCell,
  });
  await store.hsetJson(cellSummariesKey, resultId, summary);
  await store.expire(cellSummariesKey, ttl);

  const summaries = entries.filter((e) => !evicted.has(e.field) && e.field !== resultId).map((e) => e.value);
  summaries.push(summary);
  const aggregate = computeCellAggregate(summaries, {
    minAccounts: config.minAccounts,
    varianceBand: config.varianceBand,
    conforming,
  });
  const previous = await store.getJson(KEYS.cell(cellId));
  await store.setJson(
    KEYS.cell(cellId),
    {
      cell_id: cellId,
      slug,
      skill_content_hash: contentHash,
      suite_id: suite.id,
      suite_version: suite.version,
      harness,
      environment_cell: cell,
      first_seen_at: (previous && previous.first_seen_at) || summary.received_at,
      updated_at: summary.received_at,
      // Surfaced on the public read: retention pressure is part of reading the
      // evidence honestly, and the previous `truncated` flag had no reader at all.
      n_evicted_results: Number((previous && previous.n_evicted_results) || 0) + evicted.size,
      ...aggregate,
    },
    ttl,
  );

  // Written last: until this exists, a retry re-runs every step above, all of
  // which are keyed writes and therefore converge instead of double-counting.
  await store.setJson(
    KEYS.result(resultId),
    {
      result_id: resultId,
      idempotency_key: idemKey,
      cell_id: cellId,
      account_id: account.account_id,
      verified,
      slug,
      skill_content_hash: contentHash,
      suite_id: suite.id,
      suite_version: suite.version,
      config_hash: configHash,
      harness,
      environment_cell: cell,
      conforming,
      tier_state: aggregate.tier_state,
      flagged: false,
      created_at: bundle.created_at,
      received_at: summary.received_at,
      // The validated projection, never the request object: `validateBundleShape`
      // checks named fields but rejects no unknown ones, so persisting the client's
      // own object stores whatever padding it smuggled in, permanently.
      bundle: normalized,
    },
    ttl,
  );

  return {
    status: 200,
    body: {
      accepted: true,
      result_id: resultId,
      tier_state: aggregate.tier_state,
      conforming,
      reason: conforming
        ? ""
        : "stored as self-reported: only Docker-isolated runs on the suite's reference environment can be corroborated",
    },
  };
}

/// Public read-back of a skill's aggregate cells. Contributor identities are
/// never part of this payload — only counts.
///
/// `metrics_basis` names the population the rates describe, and it always matches
/// `tier_state`; the anonymous counts sit beside it so an auto-provisioned
/// install's contribution stays visible without steering a corroborated number.
export async function readSkillCells(store, slug) {
  const ids = await store.smembers(KEYS.skillCells(slug));
  if (ids.length === 0) return [];
  const metas = await store.mgetJson(ids.map((id) => KEYS.cell(id)));
  return metas
    .filter((m) => m && typeof m === "object")
    .map((m) => ({
      cell_id: m.cell_id,
      skill_content_hash: m.skill_content_hash,
      suite_id: m.suite_id,
      suite_version: m.suite_version,
      harness: m.harness || { name: "", version: "" },
      environment_cell: m.environment_cell,
      tier_state: m.tier_state,
      conforming: Boolean(m.conforming),
      metrics_basis: m.metrics_basis || "all_accounts",
      n_accounts: m.n_accounts || 0,
      n_verified_accounts: m.n_verified_accounts || 0,
      n_anonymous_accounts: m.n_anonymous_accounts || 0,
      n_anonymous_results: m.n_anonymous_results || 0,
      n_results: m.n_results || 0,
      n_trials: m.n_trials || 0,
      control_pass_rate: m.control_pass_rate ?? 0,
      treatment_pass_rate: m.treatment_pass_rate ?? 0,
      success_delta_pct: m.success_delta_pct ?? 0,
      token_delta_pct: m.token_delta_pct ?? 0,
      flagged_results: m.n_flagged_results || 0,
      evicted_results: m.n_evicted_results || 0,
      first_seen_at: m.first_seen_at || "",
      updated_at: m.updated_at || "",
    }))
    .sort((a, b) => (a.suite_id + a.cell_id).localeCompare(b.suite_id + b.cell_id));
}

/// R6: a revoked account's published results are flagged for review rather than
/// silently kept. Every cell it touched is recomputed, so corroboration counts
/// and displayed numbers drop immediately.
///
/// Flagging does not shrink the hash, which is why admission counts LIVE entries
/// and evicts flagged ones first: revocation has to give a cell its capacity back,
/// or an abuser could saturate a cell and then be revoked into a permanently dead
/// one.
export async function flagAccountResults(store, accountId, { config, now }) {
  const cellIds = await store.smembers(KEYS.accountCells(accountId));
  let flagged = 0;
  for (const cellId of cellIds) {
    const key = KEYS.cellSummaries(cellId);
    const entries = await store.hgetallJson(key);
    let changed = false;
    for (const entry of entries) {
      if (entry.value.account_id !== accountId || entry.value.flagged) continue;
      entry.value = { ...entry.value, flagged: true, flagged_at: now.toISOString() };
      await store.hsetJson(key, entry.field, entry.value);
      changed = true;
      flagged += 1;
    }
    if (!changed) continue;

    const meta = await store.getJson(KEYS.cell(cellId));
    if (!meta) continue;
    const aggregate = computeCellAggregate(
      entries.map((e) => e.value),
      { minAccounts: config.minAccounts, varianceBand: config.varianceBand, conforming: meta.conforming },
    );
    await store.setJson(KEYS.cell(cellId), { ...meta, ...aggregate, updated_at: now.toISOString() });
  }
  return { cells: cellIds.length, flagged };
}
