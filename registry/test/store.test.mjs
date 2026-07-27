import assert from "node:assert/strict";
import test from "node:test";

import { Store, memoryClient, rateLimits } from "../lib/store.mjs";

function clockedStore() {
  let now = 1_000_000;
  const store = new Store(memoryClient({ now: () => now }));
  return { store, advance: (seconds) => (now += seconds * 1000) };
}

test("the fixed-window limiter allows up to the budget then answers with a retry hint", async () => {
  const { store, advance } = clockedStore();

  for (let i = 0; i < 3; i += 1) {
    const hit = await store.hit("bucket", 3, 60);
    assert.equal(hit.allowed, true, `request ${i + 1} should be allowed`);
  }

  const denied = await store.hit("bucket", 3, 60);
  assert.equal(denied.allowed, false);
  // The CLI renders this verbatim as "retry after N seconds", so it must be a
  // whole number of seconds, never an HTTP date.
  assert.equal(Number.isInteger(denied.retryAfter), true);
  assert.ok(denied.retryAfter > 0 && denied.retryAfter <= 60);

  advance(61);
  assert.equal((await store.hit("bucket", 3, 60)).allowed, true, "the window must roll over");
});

test("buckets are independent", async () => {
  const { store } = clockedStore();
  await store.hit("a", 1, 60);
  assert.equal((await store.hit("a", 1, 60)).allowed, false);
  assert.equal((await store.hit("b", 1, 60)).allowed, true);
});

test("json round-trips through the store", async () => {
  const { store } = clockedStore();
  await store.setJson("k", { a: 1, nested: { b: [1, 2] } });
  assert.deepEqual(await store.getJson("k"), { a: 1, nested: { b: [1, 2] } });
  assert.equal(await store.getJson("missing"), null);

  await store.del("k");
  assert.equal(await store.getJson("k"), null);
});

test("hash entries come back as field/value pairs so they can be written back", async () => {
  const { store } = clockedStore();
  await store.hsetJson("h", "one", { n: 1 });
  await store.hsetJson("h", "two", { n: 2 });
  assert.equal(await store.hlen("h"), 2);

  const entries = await store.hgetallJson("h");
  entries.sort((a, b) => a.field.localeCompare(b.field));
  assert.deepEqual(entries, [
    { field: "one", value: { n: 1 } },
    { field: "two", value: { n: 2 } },
  ]);

  await store.hsetJson("h", "one", { ...entries[0].value, n: 9 });
  assert.equal(await store.hlen("h"), 2, "rewriting a field must not append");
});

test("sets deduplicate, which is what makes replay harmless", async () => {
  const { store } = clockedStore();
  await store.sadd("s", "x");
  await store.sadd("s", "x");
  await store.sadd("s", "y");
  assert.deepEqual((await store.smembers("s")).sort(), ["x", "y"]);
  await store.srem("s", "x");
  assert.deepEqual(await store.smembers("s"), ["y"]);
});

test("mgetJson tolerates missing keys and an empty request", async () => {
  const { store } = clockedStore();
  await store.setJson("a", { v: 1 });
  assert.deepEqual(await store.mgetJson([]), []);
  assert.deepEqual(await store.mgetJson(["a", "nope"]), [{ v: 1 }, null]);
});

test("rate limits fall back to defaults and accept overrides", () => {
  const defaults = rateLimits({});
  assert.equal(defaults.publishesPerAccountPerHour, 30);
  assert.equal(defaults.anonTokensPerIpPerDay, 20);

  const overridden = rateLimits({ EVAL_WRITES_PER_ACCOUNT_PER_HOUR: "5", AUTH_ANON_TOKENS_PER_IP_PER_DAY: "nonsense" });
  assert.equal(overridden.publishesPerAccountPerHour, 5);
  assert.equal(overridden.anonTokensPerIpPerDay, 20, "garbage falls back rather than disabling the limit");
});
