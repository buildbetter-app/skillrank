import assert from "node:assert/strict";
import test from "node:test";

import { createHash } from "node:crypto";

import { Store, clientIpBucket, ipSalt, memoryClient, rateLimits } from "../lib/store.mjs";

const sha256Hex = (v) => createHash("sha256").update(v, "utf8").digest("hex");

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

test("setJsonIfAbsent is a real create-if-absent, not a read-then-write", async () => {
  const { store } = clockedStore();

  const winners = await Promise.all(
    Array.from({ length: 16 }, (_, i) => store.setJsonIfAbsent("claim", { who: i })),
  );
  assert.equal(winners.filter(Boolean).length, 1, "exactly one caller may win a singleton claim");

  // The stored value must be the winner's, and a later claim must not overwrite it.
  const held = await store.getJson("claim");
  assert.equal(winners.indexOf(true), held.who);
  assert.equal(await store.setJsonIfAbsent("claim", { who: 99 }), false);
  assert.deepEqual(await store.getJson("claim"), held);
});

test("the rate-limit IP digest is not reversible from a known salt", async () => {
  // The digest IS the persisted `rl:` key name, so a hardcoded public salt makes
  // the whole `rl:` keyspace a lookup table back to plaintext IPs: SHA-256 over a
  // known constant is a 2^32 preimage search for IPv4. This used to fall back to
  // the literal salt "skillrank".
  const ip = "203.0.113.7";
  const req = { headers: { "x-forwarded-for": `${ip}, 10.0.0.1` } };

  const forgeable = ["skillrank", "", "skillrank-ip-salt"].map((salt) => sha256Hex(`${salt}:${ip}`).slice(0, 32));
  forgeable.push(sha256Hex(ip).slice(0, 32));

  for (const env of [{}, { KV_REST_API_TOKEN: "upstash-secret" }, { UPSTASH_REDIS_REST_TOKEN: "upstash-secret" }]) {
    const bucket = clientIpBucket(req, env);
    assert.match(bucket, /^[0-9a-f]{32}$/);
    for (const guess of forgeable) {
      assert.notEqual(bucket, guess, "a publicly-guessable salt must never produce the persisted key");
    }
    assert.equal(bucket.includes(ip), false, "the address itself is never in the key");
  }

  // Deriving from the deployment secret keeps counters stable across invocations,
  // which is what makes a secret salt usable at all.
  const env = { KV_REST_API_TOKEN: "upstash-secret" };
  assert.equal(clientIpBucket(req, env), clientIpBucket(req, env));
  assert.notEqual(ipSalt(env), "upstash-secret", "the credential itself is not the salt");
  assert.notEqual(clientIpBucket(req, env), clientIpBucket(req, { KV_REST_API_TOKEN: "other-secret" }));

  // Explicitly pinned wins, so the salt can be rotated.
  assert.equal(ipSalt({ RATE_LIMIT_IP_SALT: " pinned ", KV_REST_API_TOKEN: "upstash-secret" }), "pinned");

  // With no datastore the salt is a per-process random value — every write route
  // answers 503 before a limiter runs, so no digest is ever persisted.
  assert.ok(ipSalt({}).length >= 32);
  assert.notEqual(ipSalt({}), "skillrank");

  // Distinct callers still land in distinct buckets, so the limiter still limits.
  assert.notEqual(
    clientIpBucket({ headers: { "x-forwarded-for": "198.51.100.1" } }, env),
    clientIpBucket({ headers: { "x-forwarded-for": "198.51.100.2" } }, env),
  );
  // Vercel's own header wins over a client-supplied one, and the socket is the
  // last resort.
  assert.equal(
    clientIpBucket({ headers: { "x-vercel-forwarded-for": ip, "x-forwarded-for": "1.1.1.1" } }, env),
    clientIpBucket(req, env),
  );
  assert.equal(
    clientIpBucket({ headers: {}, socket: { remoteAddress: ip } }, env),
    clientIpBucket(req, env),
  );
});

test("rate limits fall back to defaults and accept overrides", () => {
  const defaults = rateLimits({});
  assert.equal(defaults.publishesPerAccountPerHour, 30);
  assert.equal(defaults.anonTokensPerIpPerDay, 20);

  const overridden = rateLimits({ EVAL_WRITES_PER_ACCOUNT_PER_HOUR: "5", AUTH_ANON_TOKENS_PER_IP_PER_DAY: "nonsense" });
  assert.equal(overridden.publishesPerAccountPerHour, 5);
  assert.equal(overridden.anonTokensPerIpPerDay, 20, "garbage falls back rather than disabling the limit");
});
