import assert from "node:assert/strict";
import test from "node:test";

import { flagAccountResults, ingestBundle, ingestConfig, readSkillCells } from "../lib/ingest.mjs";
import { KEYS, Store, memoryClient } from "../lib/store.mjs";
import { CATALOG, SLUG, SUITES, account, makeBundle } from "./helpers.mjs";

const CONFIG = ingestConfig({});
const NOW = new Date("2026-07-24T18:00:00Z");

function newStore() {
  return new Store(memoryClient());
}

function publish(store, bundle, who, now = NOW) {
  return ingestBundle({ store, bundle, account: who, suites: SUITES, catalog: CATALOG, config: CONFIG, now });
}

function freshBundle(extra = {}) {
  return makeBundle({ createdAt: "2026-07-24T17:00:00Z", ...extra });
}

test("a valid bundle is accepted and stored as self-reported", async () => {
  const store = newStore();
  const res = await publish(store, freshBundle(), account("acct_a"));

  assert.equal(res.status, 200);
  assert.equal(res.body.accepted, true);
  assert.match(res.body.result_id, /^res_[0-9a-f]{32}$/);
  assert.equal(res.body.tier_state, "self_reported");
  assert.equal(res.body.conforming, false);
  assert.match(res.body.reason, /self-reported/);

  const stored = await store.getJson(KEYS.result(res.body.result_id));
  assert.equal(stored.account_id, "acct_a");
  assert.equal(stored.slug, SLUG);
  assert.deepEqual(stored.bundle.trials.length, 6, "the raw evidence is kept for review");
});

test("republishing the identical bundle returns the original id and inflates nothing", async () => {
  const store = newStore();
  const bundle = freshBundle();
  const first = await publish(store, bundle, account("acct_a"));
  const second = await publish(store, bundle, account("acct_a"));

  assert.equal(second.status, 200);
  assert.equal(second.body.accepted, true);
  assert.equal(second.body.result_id, first.body.result_id);
  assert.equal(second.body.duplicate, true);

  const [cell] = await readSkillCells(store, SLUG);
  assert.equal(cell.n_results, 1);
  assert.equal(cell.n_accounts, 1);
  assert.equal(cell.n_trials, 6);
});

test("a second account on the same config is a second result, not a duplicate", async () => {
  const store = newStore();
  const bundle = freshBundle();
  const first = await publish(store, bundle, account("acct_a"));
  const second = await publish(store, bundle, account("acct_b"));

  assert.notEqual(first.body.result_id, second.body.result_id);
  const [cell] = await readSkillCells(store, SLUG);
  assert.equal(cell.n_accounts, 2);
  assert.equal(cell.n_results, 2);
});

test("rejections are 400 with a human-readable reason the CLI can print", async () => {
  const store = newStore();
  const res = await publish(store, freshBundle({ slug: "someone/not-in-the-catalog" }), account("acct_a"));
  assert.equal(res.status, 400);
  assert.equal(res.body.accepted, false);
  assert.equal(res.body.error, res.body.reason);
  assert.match(res.body.reason, /unknown skill/);
});

test("a content hash that is not a published version is rejected", async () => {
  const store = newStore();
  const res = await publish(store, freshBundle({ contentHash: `sha256:${"b".repeat(64)}` }), account("acct_a"));
  assert.equal(res.status, 400);
  assert.match(res.body.reason, /does not match a published version/);
});

test("a hash this registry already published stays valid after the catalog moves on", async () => {
  const store = newStore();
  const oldHash = `sha256:${"c".repeat(64)}`;
  // Simulate: results were accepted against `oldHash` before the catalog refresh.
  await store.sadd(KEYS.skillHashes(SLUG), oldHash);
  const rolled = { hasSkill: CATALOG.hasSkill, publishedHash: () => `sha256:${"d".repeat(64)}` };

  const res = await ingestBundle({
    store,
    bundle: freshBundle({ contentHash: oldHash }),
    account: account("acct_a"),
    suites: SUITES,
    catalog: rolled,
    config: CONFIG,
    now: NOW,
  });
  assert.equal(res.body.accepted, true);
});

async function corroborate(store, { kind, isolation, treatmentPass = [2, 2, 2] }) {
  const ids = [];
  for (let i = 0; i < treatmentPass.length; i += 1) {
    const who = account(`acct_${i}`, kind);
    const bundle = freshBundle({
      isolation,
      treatmentPass: treatmentPass[i],
      createdAt: `2026-07-24T1${i}:00:00Z`,
    });
    const res = await publish(store, bundle, who);
    assert.equal(res.body.accepted, true, res.body.reason);
    ids.push(res);
  }
  return ids;
}

test("three verified accounts on a conforming cell reach community-reported", async () => {
  const store = newStore();
  const results = await corroborate(store, { kind: "github", isolation: "docker" });

  assert.equal(results.at(-1).body.tier_state, "community_reported");
  const [cell] = await readSkillCells(store, SLUG);
  assert.equal(cell.tier_state, "community_reported");
  assert.equal(cell.n_verified_accounts, 3);
  assert.equal(cell.conforming, true);
});

test("anonymous accounts never corroborate, however many there are", async () => {
  const store = newStore();
  const results = await corroborate(store, {
    kind: "anonymous",
    isolation: "docker",
    treatmentPass: [2, 2, 2, 2, 2, 2],
  });

  // This is the whole reason two token classes exist: auto-provisioned identity
  // must not make "N independent accounts" mean "N installs".
  assert.equal(results.at(-1).body.tier_state, "self_reported");
  const [cell] = await readSkillCells(store, SLUG);
  assert.equal(cell.n_accounts, 6);
  assert.equal(cell.n_verified_accounts, 0);
  assert.equal(cell.tier_state, "self_reported");
});

test("non-conforming cells stay self-reported no matter how many verified accounts agree", async () => {
  const store = newStore();
  const results = await corroborate(store, { kind: "github", isolation: "worktree" });
  assert.equal(results.at(-1).body.tier_state, "self_reported");
  const [cell] = await readSkillCells(store, SLUG);
  assert.equal(cell.n_verified_accounts, 3);
  assert.equal(cell.conforming, false);
  assert.equal(cell.tier_state, "self_reported");
});

test("results that disagree beyond the variance band stay self-reported", async () => {
  const store = newStore();
  // Deltas of +2/3, 0 and -1/3 span 1.0 — far outside the 0.25 band.
  const results = await corroborate(store, { kind: "github", isolation: "docker", treatmentPass: [3, 1, 0] });
  assert.equal(results.at(-1).body.tier_state, "self_reported");
  const [cell] = await readSkillCells(store, SLUG);
  assert.ok(cell.n_verified_accounts >= 3);
  assert.equal(cell.tier_state, "self_reported");
});

test("revoking an account flags its results and drops the cell back to self-reported", async () => {
  const store = newStore();
  await corroborate(store, { kind: "github", isolation: "docker" });
  assert.equal((await readSkillCells(store, SLUG))[0].tier_state, "community_reported");

  const flagged = await flagAccountResults(store, "acct_1", { config: CONFIG, now: NOW });
  assert.equal(flagged.flagged, 1);

  const [cell] = await readSkillCells(store, SLUG);
  assert.equal(cell.tier_state, "self_reported");
  assert.equal(cell.n_verified_accounts, 2);
  assert.equal(cell.flagged_results, 1);
});

test("aggregate cells expose counts but never contributor identities", async () => {
  const store = newStore();
  await publish(store, freshBundle(), account("acct_secret"));
  const cells = await readSkillCells(store, SLUG);
  assert.equal(cells.length, 1);
  assert.equal(JSON.stringify(cells).includes("acct_secret"), false);
  assert.equal(cells[0].success_delta_pct, 33.3);
  assert.equal(cells[0].token_delta_pct, -10);
  assert.equal(cells[0].suite_id, "debug-basics");
});

test("different environment cells do not share a rollup", async () => {
  const store = newStore();
  await publish(store, freshBundle({ isolation: "docker" }), account("acct_a"));
  await publish(store, freshBundle({ isolation: "worktree" }), account("acct_a"));
  const cells = await readSkillCells(store, SLUG);
  assert.equal(cells.length, 2);
  assert.deepEqual(
    cells.map((c) => c.environment_cell.isolation).sort(),
    ["docker", "worktree"],
  );
});

test("a partially failed write converges on retry instead of double counting", async () => {
  const store = newStore();
  const bundle = freshBundle();
  const first = await publish(store, bundle, account("acct_a"));

  // Simulate the store dying right before the final result record was written.
  await store.del(KEYS.result(first.body.result_id));
  const retry = await publish(store, bundle, account("acct_a"));

  assert.equal(retry.body.result_id, first.body.result_id);
  const [cell] = await readSkillCells(store, SLUG);
  assert.equal(cell.n_results, 1);
  assert.equal(cell.n_trials, 6);
});
