// Discovery reads: `GET /eval-suites` (the suite index) and `GET /skills/facets`
// (the catalog's real filter vocabulary). Both exist so a client stops guessing —
// suite ids used to be typeable-from-memory only, and filter option lists used to
// be hardcoded against values no skill carries.
//
// Like route.test.mjs these boot the REAL function behind node:http, because the
// facets route's correctness is partly a ROUTING property: `/skills/facets` must
// not be swallowed by the slug parser that joins every remaining path segment.

import assert from "node:assert/strict";
import { createServer } from "node:http";
import test, { after, before } from "node:test";

const { default: handler } = await import("../api/registry.mjs");

let server;
let base;

before(async () => {
  server = createServer((req, res) => handler(req, res));
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  base = `http://127.0.0.1:${server.address().port}`;
});

after(() => server.close());

/// Mirrors `registry/vercel.json`: /v3/rest/skill-registry/:path* -> ?path=:path*
async function call(path, params = {}, init = {}) {
  const res = await fetch(`${base}/api/registry?${new URLSearchParams({ path, ...params })}`, init);
  return { status: res.status, headers: res.headers, body: await res.json().catch(() => null) };
}

// ---- GET /eval-suites ----------------------------------------------------

test("the eval-suite index lists every suite", async () => {
  const res = await call("eval-suites");
  assert.equal(res.status, 200);
  assert.ok(Array.isArray(res.body.items));
  assert.ok(res.body.items.length > 0, "the registry ships at least one suite");
  assert.equal(res.body.total, res.body.items.length);

  for (const item of res.body.items) {
    assert.ok(item.id, "every row must carry the id a client passes to `eval`");
    assert.ok(item.version, "and the version that pins the suite");
    assert.ok(Number.isInteger(item.task_count) && item.task_count > 0);
  }
});

test("the index stays lightweight: no task bodies", async () => {
  const res = await call("eval-suites");
  const serialized = JSON.stringify(res.body);
  for (const item of res.body.items) {
    assert.equal(item.tasks, undefined, "task arrays belong to the detail route");
  }
  // The instruction of the first suite's first task is the single biggest field
  // in a suite; if it leaks into the index, "lightweight" is not true.
  const detail = (await call(`eval-suites/${res.body.items[0].id}`)).body;
  assert.ok(detail.tasks[0].instruction.length > 40);
  assert.equal(serialized.includes(detail.tasks[0].instruction), false);
});

test("every indexed suite is fetchable at the id and version it advertises", async () => {
  const index = await call("eval-suites");
  for (const item of index.body.items) {
    const detail = await call(`eval-suites/${item.id}`);
    assert.equal(detail.status, 200, `${item.id} must resolve`);
    assert.equal(detail.body.id, item.id);
    assert.equal(detail.body.version, item.version);
    assert.equal(detail.body.tasks.length, item.task_count, `${item.id} task_count must be honest`);
    assert.deepEqual(item.reference_env, detail.body.reference_env);
  }
});

test("the suite index is an anonymous, cacheable read", async () => {
  const anonymous = await call("eval-suites");
  assert.match(anonymous.headers.get("cache-control"), /s-maxage/);
  const withToken = await call("eval-suites", {}, { headers: { Authorization: "Bearer srk_aaaaaaaaaaaaaaaaaaaaaaaa" } });
  assert.deepEqual(withToken.body, anonymous.body);
});

test("adding the index did not turn an unknown suite id into a list", async () => {
  const missing = await call("eval-suites/no-such-suite");
  assert.equal(missing.status, 404);
  assert.equal(missing.body.error, "suite not found");
});

// ---- GET /skills/facets --------------------------------------------------

test("facets report the catalog's taxonomy, sorted by count desc", async () => {
  const res = await call("skills/facets");
  assert.equal(res.status, 200);
  for (const key of ["categories", "stacks", "scan_tiers"]) {
    const rows = res.body[key];
    assert.ok(Array.isArray(rows) && rows.length > 0, `${key} must be a non-empty list`);
    for (const row of rows) {
      assert.equal(typeof row.value, "string");
      assert.ok(row.value.length > 0, `${key} must never offer an empty filter value`);
      assert.equal(row.value, row.value.toLowerCase(), "values must be comparable to a lowercased query");
      assert.ok(Number.isInteger(row.count) && row.count > 0);
    }
    const counts = rows.map((r) => r.count);
    assert.deepEqual(counts, [...counts].sort((a, b) => b - a), `${key} must be ordered by count desc`);
  }
});

test("every facet count is the total that filtering on it actually returns", async () => {
  const facets = (await call("skills/facets")).body;
  for (const row of facets.categories.slice(0, 5)) {
    const search = await call("skills", { category: row.value, limit: "1" });
    assert.equal(search.body.total, row.count, `category=${row.value}`);
  }
  for (const row of facets.stacks.slice(0, 5)) {
    const search = await call("skills", { stack: row.value, limit: "1" });
    assert.equal(search.body.total, row.count, `stack=${row.value}`);
  }
});

test("the taxonomy is the real one: a value it omits matches nothing", async () => {
  const facets = (await call("skills/facets")).body;
  const stacks = new Map(facets.stacks.map((r) => [r.value, r.count]));
  // This is the vocabulary a client hardcoded as its stack filter. Most of it
  // matches no skill, which is exactly the failure facets exist to end: whatever
  // the catalog happens to hold, absent-from-facets must mean zero results, and
  // present must mean the advertised count.
  const guessed = ["nextjs", "react", "typescript", "playwright", "vitest", "shadcn", "tailwind", "node-api", "svelte", "vue", "python", "fastapi", "django", "rails", "go", "rust", "java"];
  let absent = 0;
  for (const value of guessed) {
    const total = (await call("skills", { stack: value, limit: "1" })).body.total;
    assert.equal(total, stacks.get(value) ?? 0, `stack=${value}`);
    if (total === 0) absent += 1;
  }
  assert.ok(absent > 0, "the guessed vocabulary is supposed to contain dead options");
});

test("scan-tier facets partition the whole catalog", async () => {
  const facets = (await call("skills/facets")).body;
  const all = await call("skills", { limit: "1" });
  const summed = facets.scan_tiers.reduce((n, row) => n + row.count, 0);
  // Every entry has exactly one scan tier, so the tiers must add up to the
  // unfiltered catalog size — a facet that under-counts would hide skills.
  assert.equal(summed, all.body.total);
});

test("/skills/facets is not parsed as a slug", async () => {
  const facets = await call("skills/facets");
  assert.equal(facets.status, 200);
  assert.equal(facets.body.error, undefined);
  // The reservation is exactly one path: sub-paths under it are still slug reads,
  // and a real multi-segment slug is untouched.
  assert.equal((await call("skills/facets/resolve")).status, 404);
  assert.equal((await call("skills/obra/systematic-debugging")).status, 200);
});

test("facets are an anonymous, cacheable read", async () => {
  const anonymous = await call("skills/facets");
  assert.match(anonymous.headers.get("cache-control"), /s-maxage/);
  const withToken = await call("skills/facets", {}, { headers: { Authorization: "Bearer srk_aaaaaaaaaaaaaaaaaaaaaaaa" } });
  assert.deepEqual(withToken.body, anonymous.body);
});
