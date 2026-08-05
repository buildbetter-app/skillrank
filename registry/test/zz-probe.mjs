import assert from "node:assert/strict";
import { computeCellAggregate, ingestBundle, ingestConfig, flagAccountResults, readSkillCells } from "../lib/ingest.mjs";
import { KEYS, Store, memoryClient } from "../lib/store.mjs";
import { upsertVerifiedAccount } from "../lib/auth.mjs";
import { CATALOG, SLUG, SUITES, account, makeBundle } from "./helpers.mjs";

const NOW = new Date("2026-07-24T18:00:00Z");
const CONFIG = ingestConfig({});
const fresh = (extra = {}) => makeBundle({ createdAt: "2026-07-24T17:00:00Z", ...extra });

function newStore(cfg) {
  const client = memoryClient();
  return { store: new Store(client), client };
}
const publish = (store, bundle, who, config = CONFIG) =>
  ingestBundle({ store, bundle, account: who, suites: SUITES, catalog: CATALOG, config, now: NOW });

// ---------- F1: mixed-class cell ----------
{
  const { store } = newStore();
  for (let i = 0; i < 3; i += 1) {
    const r = await publish(store, fresh({ isolation: "docker", createdAt: `2026-07-24T1${i}:00:00Z` }), account(`v${i}`, "github"));
    assert.equal(r.body.accepted, true, r.body.reason);
  }
  let [cell] = await readSkillCells(store, SLUG);
  console.log("F1 before flood:", cell.tier_state, cell.success_delta_pct, cell.token_delta_pct, cell.metrics_basis);
  // one anonymous token floods 100 bundles with catastrophic numbers
  let accepted = 0;
  for (let i = 0; i < 100; i += 1) {
    const b = fresh({ isolation: "docker", controlPass: 3, treatmentPass: 0, controlTokens: 1000, treatmentTokens: 200000, createdAt: `2026-07-2${(i % 4) + 1}T0${i % 10}:0${i % 6}:00Z` });
    const r = await publish(store, b, account("anon", "anonymous"));
    if (r.body.accepted) accepted += 1;
  }
  [cell] = await readSkillCells(store, SLUG);
  console.log("F1 after flood:", { accepted, tier: cell.tier_state, delta: cell.success_delta_pct, tok: cell.token_delta_pct, basis: cell.metrics_basis, n_anon: cell.n_anonymous_accounts, n_anon_res: cell.n_anonymous_results, n_verified: cell.n_verified_accounts });
}

// ---------- F2/F6: saturated cell silently drops but returns accepted:true ----------
{
  const cfg = ingestConfig({ EVAL_MAX_RESULTS_PER_CELL: "4", EVAL_MAX_RESULTS_PER_ACCOUNT_PER_CELL: "2" });
  const { store, client } = newStore();
  const ids = [];
  for (let i = 0; i < 8; i += 1) {
    const r = await publish(store, fresh({ isolation: "docker", createdAt: `2026-07-24T0${i}:00:00Z` }), account(`v${i}`, "github"), cfg);
    ids.push([r.body.accepted, r.body.result_id]);
  }
  const live = await store.hgetallJson(KEYS.cellSummaries((await store.smembers(KEYS.skillCells(SLUG)))[0]));
  console.log("F2 accepted count:", ids.filter(([a]) => a).length, "retained:", live.length, "cap: 4");
  const lastId = ids.at(-1)[1];
  console.log("F2 last submission retained?", live.some((e) => e.field === lastId));
}

// ---------- F3: created_at is the only free variable ----------
{
  const { store } = newStore();
  let n = 0;
  for (let i = 0; i < 40; i += 1) {
    const r = await publish(store, fresh({ createdAt: `2026-07-24T${String(i % 24).padStart(2, "0")}:${String(i).padStart(2, "0")}:00Z` }), account("solo", "github"));
    if (r.body.accepted && !r.body.duplicate) n += 1;
  }
  const cellId = (await store.smembers(KEYS.skillCells(SLUG)))[0];
  console.log("F3 distinct accepted:", n, "slots held by one account:", (await store.hgetallJson(KEYS.cellSummaries(cellId))).length);
}

// ---------- F4/5/8: concurrent first login mints N accounts ----------
{
  const { store } = newStore();
  const results = await Promise.all(
    Array.from({ length: 8 }, () => upsertVerifiedAccount(store, { provider: "github", subject: "777", now: NOW.toISOString() })),
  );
  const ids = new Set(results.map((r) => r.account && r.account.account_id).filter(Boolean));
  console.log("F4 distinct account ids for one identity:", ids.size, "conflicts:", results.filter((r) => r.conflict).length);
}

// ---------- F7: cell cardinality + TTL ----------
{
  const cfg = ingestConfig({ EVAL_MAX_CELLS_PER_SKILL: "3", EVAL_NEW_CELLS_PER_ACCOUNT_PER_DAY: "100" });
  const { store, client } = newStore();
  let made = 0;
  for (let i = 0; i < 10; i += 1) {
    const r = await publish(store, fresh({ model: `m${i}` }), account("v", "github"), cfg);
    if (r.body.accepted) made += 1;
  }
  const cells = await store.smembers(KEYS.skillCells(SLUG));
  console.log("F7 cells minted:", made, "cells stored:", cells.length);
  console.log("F7 ttl eval:cell:*:", await client.ttl(KEYS.cell(cells[0])), "summaries:", await client.ttl(KEYS.cellSummaries(cells[0])), "skillCells:", await client.ttl(KEYS.skillCells(SLUG)));
}

// ---------- F7b: TTL after revocation flagging ----------
{
  const { store, client } = newStore();
  for (let i = 0; i < 3; i += 1) {
    await publish(store, fresh({ isolation: "docker", createdAt: `2026-07-24T1${i}:00:00Z` }), account(`v${i}`, "github"));
  }
  const cellId = (await store.smembers(KEYS.skillCells(SLUG)))[0];
  console.log("F7b ttl before flag:", await client.ttl(KEYS.cell(cellId)));
  await flagAccountResults(store, "v1", { config: CONFIG, now: NOW });
  console.log("F7b ttl after flag:", await client.ttl(KEYS.cell(cellId)), "(-1 means immortal)");
}

// ---------- F9: raw bundle padding + TTL ----------
{
  const { store, client } = newStore();
  const b = fresh();
  b.padding = "P".repeat(50_000);
  b.trials[0].secret_padding = "Q".repeat(10_000);
  const r = await publish(store, b, account("v", "github"));
  const stored = await store.getJson(KEYS.result(r.body.result_id));
  console.log("F9 accepted:", r.body.accepted, "padding persisted:", JSON.stringify(stored).includes("PPPP"), "trial padding:", JSON.stringify(stored).includes("QQQQ"), "ttl:", await client.ttl(KEYS.result(r.body.result_id)));
}

// ---------- F13: harness version ----------
{
  const { store } = newStore();
  const junk = await publish(store, fresh({ harnessVersion: "hacked-by-me" }), account("v", "github"));
  console.log("F13 junk harness version accepted:", junk.body.accepted, "|", junk.body.reason);
  const a = await publish(store, fresh({ isolation: "docker", harnessVersion: "0.1.0" }), account("v1", "github"));
  const bb = await publish(store, fresh({ isolation: "docker", harnessVersion: "9.9.9" }), account("v2", "github"));
  console.log("F13 cells for two harness versions:", (await store.smembers(KEYS.skillCells(SLUG))).length);
}

// ---------- F11: conforming from client-supplied fields ----------
{
  const { store } = newStore();
  // A worktree run relabelled as docker, with config_hash recomputed to agree.
  const r = await publish(store, fresh({ isolation: "docker" }), account("v", "github"));
  console.log("F11 self-attested docker accepted as conforming:", r.body.conforming);
}
console.log("PROBE DONE");
