// Regression tests for the abuse paths an xhigh review found exploitable.
//
// Every test here reproduces a specific published exploit and asserts the exploit
// DOES NOT WORK — not merely that the function returns something. Where the
// original attack is only interesting because the honest path exists, both are
// asserted, so a fix that neuters the feature fails too.
//
// The one thing the original suite never did was put more than one account CLASS
// in a single cell: it corroborated with three github accounts, or flooded with
// six anonymous ones, never both. That is why the aggregate could be authored by
// free anonymous tokens while the badge was earned by verified ones, and nothing
// caught it. The first test below is that mixed-class cell.

import assert from "node:assert/strict";
import test from "node:test";

import {
  computeCellAggregate,
  flagAccountResults,
  ingestBundle,
  ingestConfig,
  readSkillCells,
} from "../lib/ingest.mjs";
import { KEYS, Store, memoryClient } from "../lib/store.mjs";
import { CATALOG, SLUG, SUITES, account, makeBundle } from "./helpers.mjs";

const NOW = new Date("2026-07-24T18:00:00Z");
const CONFIG = ingestConfig({});

function newStore() {
  const client = memoryClient();
  return { store: new Store(client), client };
}

const fresh = (extra = {}) => makeBundle({ createdAt: "2026-07-24T17:00:00Z", ...extra });

function publish(store, bundle, who, config = CONFIG) {
  return ingestBundle({ store, bundle, account: who, suites: SUITES, catalog: CATALOG, config, now: NOW });
}

async function onlyCellId(store) {
  const ids = await store.smembers(KEYS.skillCells(SLUG));
  assert.equal(ids.length, 1, "these tests assume a single cell");
  return ids[0];
}

/// Three honest verified runs on a conforming cell: +33.3pp, -10% tokens.
async function corroborated(store, config = CONFIG) {
  for (let i = 0; i < 3; i += 1) {
    const res = await publish(
      store,
      fresh({ isolation: "docker", createdAt: `2026-07-24T1${i}:00:00Z` }),
      account(`verified_${i}`, "github"),
      config,
    );
    assert.equal(res.body.accepted, true, res.body.reason);
  }
  const [cell] = await readSkillCells(store, SLUG);
  assert.equal(cell.tier_state, "community_reported", "the honest path must still work");
  return cell;
}

// ---- 1. the badge and the numbers must describe the same population -------

test("anonymous floods cannot author the numbers under a community-reported badge", async () => {
  const { store } = newStore();
  const before = await corroborated(store);
  assert.equal(before.metrics_basis, "verified_accounts");
  assert.equal(before.success_delta_pct, 33.3);
  assert.equal(before.token_delta_pct, -10);

  // The exploit: 100 free anonymous tokens publish into the SAME cell, each
  // claiming the skill destroys success and burns 200x the tokens. Anonymous
  // accounts are excluded from earning the badge, so they must be excluded from
  // authoring the numbers printed under it.
  for (let i = 0; i < 100; i += 1) {
    const res = await publish(
      store,
      fresh({ isolation: "docker", controlPass: 3, treatmentPass: 0, controlTokens: 1000, treatmentTokens: 200_000 }),
      account(`anon_${i}`, "anonymous"),
    );
    assert.equal(res.body.accepted, true, res.body.reason);
    assert.equal(res.body.tier_state, "community_reported", "the flood must not change the tier either");
  }

  const [after] = await readSkillCells(store, SLUG);
  assert.equal(after.tier_state, "community_reported");
  assert.equal(after.metrics_basis, "verified_accounts");
  assert.equal(after.success_delta_pct, 33.3, "the headline must still be the verified population's");
  assert.equal(after.token_delta_pct, -10);
  assert.equal(after.n_verified_accounts, 3);
  assert.equal(after.n_accounts, 3, "n_accounts describes the basis population");
  assert.equal(after.n_results, 3);
  // Not hidden — reported beside the headline as its own count.
  assert.equal(after.n_anonymous_accounts, 100);
  assert.equal(after.n_anonymous_results, 100);

  // Proof the payload was potent: pooling the same summaries over all accounts
  // (what the broken aggregate did) moves +33.3pp to below -90pp. If someone
  // reverts the basis split, the assertions above start reading these numbers.
  const summaries = (await store.hgetallJson(KEYS.cellSummaries(await onlyCellId(store)))).map((e) => e.value);
  const pooled = computeCellAggregate(summaries, { minAccounts: 999, varianceBand: 0.25, conforming: true });
  assert.equal(pooled.metrics_basis, "all_accounts");
  assert.ok(pooled.success_delta_pct < -90, `expected a wrecked delta, got ${pooled.success_delta_pct}`);
  assert.ok(pooled.token_delta_pct > 1000, `expected a wrecked token delta, got ${pooled.token_delta_pct}`);
});

test("a self-reported cell says so and its numbers do cover every account", async () => {
  // The same invariant in the other direction: when the badge is Self-reported,
  // "whoever reported it" is the honest population, so anonymous results are IN
  // the rates — and `metrics_basis` still names exactly who is being described.
  const { store } = newStore();
  for (let i = 0; i < 2; i += 1) {
    await publish(store, fresh({ isolation: "docker", createdAt: `2026-07-24T1${i}:00:00Z` }), account(`v${i}`, "github"));
  }
  for (let i = 0; i < 5; i += 1) {
    await publish(store, fresh({ isolation: "docker", controlPass: 3, treatmentPass: 0 }), account(`a${i}`, "anonymous"));
  }

  const [cell] = await readSkillCells(store, SLUG);
  assert.equal(cell.tier_state, "self_reported", "5 anonymous accounts cannot make up for 1 missing verified one");
  assert.equal(cell.n_verified_accounts, 2);
  assert.equal(cell.metrics_basis, "all_accounts");
  assert.equal(cell.n_accounts, 7);
  assert.ok(cell.success_delta_pct < 0, "anonymous results are visible in a self-reported headline");
});

// ---- 2 & 6. a full cell must evict, never silently drop ------------------

test("a saturated cell keeps admitting, and never answers accepted:true for a dropped result", async () => {
  const config = ingestConfig({ EVAL_MAX_RESULTS_PER_CELL: "4", EVAL_MAX_RESULTS_PER_ACCOUNT_PER_CELL: "2" });
  const { store } = newStore();

  const submitted = [];
  for (let i = 0; i < 8; i += 1) {
    const res = await publish(
      store,
      fresh({ isolation: "docker", createdAt: `2026-07-24T0${i}:00:00Z` }),
      account(`v${i}`, "github"),
      config,
    );
    assert.equal(res.body.accepted, true, res.body.reason);
    submitted.push(res.body.result_id);
  }

  const cellId = await onlyCellId(store);
  const retained = await store.hgetallJson(KEYS.cellSummaries(cellId));
  assert.equal(retained.length, 4, "the ceiling still holds");
  // The exact bug: submission 5..8 used to be skipped while the response said
  // accepted:true. Whatever we accepted last must actually be in the cell.
  assert.ok(
    retained.some((e) => e.field === submitted.at(-1)),
    "the most recent accepted submission must be retained, not silently discarded",
  );

  const [cell] = await readSkillCells(store, SLUG);
  assert.ok(cell.evicted_results > 0, "retention pressure is reported, not hidden");
});

test("flooding a cell cannot make it permanently un-corroboratable", async () => {
  // The compound exploit: fill a cell from one cheap account so later honest
  // submissions are dropped, then get revoked so the cell reports no evidence and
  // still admits nothing. Eviction order puts the biggest holder (and flagged
  // entries) first, so the flood displaces itself.
  const config = ingestConfig({ EVAL_MAX_RESULTS_PER_CELL: "6", EVAL_MAX_RESULTS_PER_ACCOUNT_PER_CELL: "6" });
  const { store } = newStore();

  for (let i = 0; i < 6; i += 1) {
    const res = await publish(
      store,
      fresh({ isolation: "docker", createdAt: `2026-07-24T0${i}:00:00Z` }),
      account("flooder", "anonymous"),
      config,
    );
    assert.equal(res.body.accepted, true, res.body.reason);
  }
  assert.equal((await store.hgetallJson(KEYS.cellSummaries(await onlyCellId(store)))).length, 6, "cell is full");

  // Revocation must give the capacity back rather than freeze the cell.
  await flagAccountResults(store, "flooder", { config, now: NOW });

  for (let i = 0; i < 3; i += 1) {
    const res = await publish(
      store,
      fresh({ isolation: "docker", createdAt: `2026-07-24T1${i}:00:00Z` }),
      account(`honest_${i}`, "github"),
      config,
    );
    assert.equal(res.body.accepted, true, res.body.reason);
  }

  const [cell] = await readSkillCells(store, SLUG);
  assert.equal(cell.tier_state, "community_reported", "honest corroboration must still be reachable");
  assert.equal(cell.n_verified_accounts, 3);
  assert.equal(cell.success_delta_pct, 33.3);
});

// ---- 3. created_at is client-chosen, so it cannot be the only bound -------

test("one account cannot occupy a cell by bumping created_at", async () => {
  const { store } = newStore();
  const share = CONFIG.maxResultsPerAccountPerCell;
  assert.ok(share < CONFIG.maxResultsPerCell, "the per-account share must be smaller than the cell");

  // 40 re-registrations of the SAME physical run, differing only in the one field
  // the idempotency tuple leaves free. Each is a fresh result_id, so the tuple
  // alone stops nothing.
  const ids = new Set();
  for (let i = 0; i < 40; i += 1) {
    const createdAt = `2026-07-24T${String(i % 24).padStart(2, "0")}:${String(i).padStart(2, "0")}:00Z`;
    const res = await publish(store, fresh({ isolation: "docker", createdAt }), account("greedy", "github"));
    assert.equal(res.body.accepted, true, res.body.reason);
    ids.add(res.body.result_id);
  }
  assert.ok(ids.size > share, "the exploit input really did mint distinct submissions");

  const held = await store.hgetallJson(KEYS.cellSummaries(await onlyCellId(store)));
  assert.equal(held.length, share, `one account must never hold more than ${share} of a cell`);

  const [cell] = await readSkillCells(store, SLUG);
  assert.equal(cell.n_accounts, 1);
  assert.equal(cell.tier_state, "self_reported", "volume from one account is not corroboration");
});

test("a flooding account cannot displace other accounts' evidence", async () => {
  const config = ingestConfig({ EVAL_MAX_RESULTS_PER_CELL: "8", EVAL_MAX_RESULTS_PER_ACCOUNT_PER_CELL: "4" });
  const { store } = newStore();
  for (let i = 0; i < 3; i += 1) {
    await publish(
      store,
      fresh({ isolation: "docker", createdAt: `2026-07-24T1${i}:00:00Z` }),
      account(`verified_${i}`, "github"),
      config,
    );
  }
  for (let i = 0; i < 20; i += 1) {
    await publish(
      store,
      fresh({ isolation: "docker", controlPass: 3, treatmentPass: 0, createdAt: `2026-07-2${(i % 4) + 1}T0${i % 10}:00:00Z` }),
      account("flooder", "anonymous"),
      config,
    );
  }

  const [cell] = await readSkillCells(store, SLUG);
  assert.equal(cell.n_verified_accounts, 3, "the honest accounts' results survived the flood");
  assert.equal(cell.tier_state, "community_reported");
  assert.equal(cell.success_delta_pct, 33.3);
});

// ---- 7. cell cardinality and retention ----------------------------------

test("cell identity is client-chosen, so its cardinality is capped per skill", async () => {
  const config = ingestConfig({ EVAL_MAX_CELLS_PER_SKILL: "3", EVAL_NEW_CELLS_PER_ACCOUNT_PER_DAY: "100" });
  const { store } = newStore();

  for (let i = 0; i < 3; i += 1) {
    const res = await publish(store, fresh({ model: `model${i}` }), account("v", "github"), config);
    assert.equal(res.body.accepted, true, res.body.reason);
  }
  const overflow = await publish(store, fresh({ model: "model99" }), account("v", "github"), config);
  assert.equal(overflow.status, 400);
  assert.equal(overflow.body.accepted, false);
  assert.match(overflow.body.reason, /already has 3 environment cells/);
  assert.equal((await store.smembers(KEYS.skillCells(SLUG))).length, 3, "the keyspace stayed bounded");

  // ...and publishing into a cell that already exists is unaffected, so the
  // ceiling is not a lockout.
  const existing = await publish(store, fresh({ model: "model0" }), account("w", "github"), config);
  assert.equal(existing.body.accepted, true, existing.body.reason);
});

test("the per-account new-cell budget stops one account minting the whole ceiling", async () => {
  const config = ingestConfig({ EVAL_MAX_CELLS_PER_SKILL: "100", EVAL_NEW_CELLS_PER_ACCOUNT_PER_DAY: "2" });
  const { store } = newStore();

  assert.equal((await publish(store, fresh({ model: "a" }), account("v", "github"), config)).body.accepted, true);
  assert.equal((await publish(store, fresh({ model: "b" }), account("v", "github"), config)).body.accepted, true);
  const denied = await publish(store, fresh({ model: "c" }), account("v", "github"), config);
  assert.equal(denied.body.accepted, false);
  assert.match(denied.body.reason, /opened 2 new environment cells/);

  // Another account has its own budget, so this is a per-account brake, not a
  // global freeze.
  assert.equal((await publish(store, fresh({ model: "c" }), account("w", "github"), config)).body.accepted, true);
});

test("every eval:* key carries a retention TTL, including after a revocation rewrite", async () => {
  const { store, client } = newStore();
  await corroborated(store);
  const cellId = await onlyCellId(store);
  const [result] = await store.hgetallJson(KEYS.cellSummaries(cellId));

  const keys = {
    cell: KEYS.cell(cellId),
    summaries: KEYS.cellSummaries(cellId),
    skillCells: KEYS.skillCells(SLUG),
    skillHashes: KEYS.skillHashes(SLUG),
    accountCells: KEYS.accountCells("verified_0"),
    result: KEYS.result(result.field),
  };
  for (const [name, key] of Object.entries(keys)) {
    assert.ok((await client.ttl(key)) > 0, `${name} (${key}) must expire; -1 means it pins storage forever`);
  }

  // Redis SET discards a key's expiry, so rewriting a cell during revocation used
  // to make every cell a revoked account had touched immortal — reachable by
  // publishing widely and then getting revoked.
  await flagAccountResults(store, "verified_1", { config: CONFIG, now: NOW });
  assert.ok(
    (await client.ttl(keys.cell)) > 0,
    "the revocation rewrite must re-arm the retention TTL, not drop it",
  );
  assert.ok((await client.ttl(keys.summaries)) > 0);
});

// ---- 9. what gets persisted is the projection, not the request -----------

test("client-supplied padding is never persisted, and the record expires", async () => {
  const { store, client } = newStore();
  const bundle = fresh();
  bundle.padding = "P".repeat(200_000);
  bundle.nested = { deep: ["Q".repeat(50_000)] };
  bundle.trials[0].sneaky = "R".repeat(50_000);

  const res = await publish(store, bundle, account("v", "github"));
  assert.equal(res.body.accepted, true, res.body.reason);

  const stored = await store.getJson(KEYS.result(res.body.result_id));
  const blob = JSON.stringify(stored);
  for (const marker of ["PPPP", "QQQQ", "RRRR", "padding", "nested", "sneaky"]) {
    assert.equal(blob.includes(marker), false, `unvalidated field "${marker}" must not be persisted`);
  }
  assert.deepEqual(Object.keys(stored.bundle).sort(), [
    "bundle_version",
    "config_hash",
    "created_at",
    "environment_cell",
    "harness",
    "skill_content_hash",
    "skill_slug",
    "suite_id",
    "suite_version",
    "trials",
  ]);
  assert.equal(stored.bundle.trials.length, 6, "the evidence itself is still kept for review");
  assert.ok((await client.ttl(KEYS.result(res.body.result_id))) > 0, "the raw evidence must expire");
});

// ---- 13. harness version is a comparability determinant ------------------

test("an arbitrary harness.version is rejected rather than minting a cell", async () => {
  const { store } = newStore();
  for (const version of ["hacked-by-me", "", "1", "99999999.0.0", "0.1.0; rm -rf /", "v0.1.0"]) {
    const res = await publish(store, fresh({ harnessVersion: version }), account("v", "github"));
    assert.equal(res.body.accepted, false, `harness.version ${JSON.stringify(version)} must not be accepted`);
    assert.match(res.body.reason, /harness/);
  }
  assert.deepEqual(await store.smembers(KEYS.skillCells(SLUG)), [], "no cell was created for a junk version");
});

test("results from different harness versions cannot corroborate each other", async () => {
  const { store } = newStore();
  // Two accounts on each of two runner versions. Pooled, that is four verified
  // accounts and Community-reported; kept apart, neither cell has three, which is
  // the honest answer because the numbers are not comparable.
  for (const [version, tag] of [
    ["0.1.0", "old"],
    ["9.9.9", "new"],
  ]) {
    for (let i = 0; i < 2; i += 1) {
      const res = await publish(
        store,
        fresh({ isolation: "docker", harnessVersion: version, createdAt: `2026-07-24T1${i}:00:00Z` }),
        account(`${tag}_${i}`, "github"),
      );
      assert.equal(res.body.accepted, true, res.body.reason);
    }
  }

  const cells = await readSkillCells(store, SLUG);
  assert.equal(cells.length, 2, "harness version must be part of cell identity");
  assert.deepEqual(
    cells.map((c) => c.harness.version).sort(),
    ["0.1.0", "9.9.9"],
  );
  for (const cell of cells) {
    assert.equal(cell.n_verified_accounts, 2);
    assert.equal(cell.tier_state, "self_reported", "incomparable runs must not corroborate");
  }
});
