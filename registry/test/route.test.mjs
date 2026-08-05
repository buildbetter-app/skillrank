// End-to-end wiring test: boots the REAL deployed function behind node:http and
// drives it with fetch, so header parsing, the Vercel rewrite's `?path=` shape,
// body limits, and cache headers are all exercised for real.
//
// The datastore is deliberately left unconfigured here, which is exactly the case
// that must never silently succeed: every write answers 503 rather than pretending
// it persisted something. Store-backed behaviour is covered in ingest.test.mjs.

import assert from "node:assert/strict";
import { createServer } from "node:http";
import test, { after, before } from "node:test";

for (const key of ["KV_REST_API_URL", "KV_REST_API_TOKEN", "UPSTASH_REDIS_REST_URL", "UPSTASH_REDIS_REST_TOKEN"]) {
  delete process.env[key];
}
process.env.EVAL_MAX_BODY_BYTES = "512";

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
async function call(path, init = {}, params = {}) {
  const res = await fetch(`${base}/api/registry?${new URLSearchParams({ path, ...params })}`, init);
  return { status: res.status, headers: res.headers, body: await res.json().catch(() => null) };
}

test("publishing without a token is 401 with a body the client can render", async () => {
  const res = await call("eval-results", { method: "POST", body: "{}" });
  assert.equal(res.status, 401);
  assert.equal(res.body.accepted, false);
  assert.equal(res.body.error, "not signed in");
  assert.match(res.body.reason, /skillrank login/);
  assert.equal(res.headers.get("cache-control"), "no-store", "auth answers must never be edge-cached");
});

test("eval-results rejects non-POST", async () => {
  const res = await call("eval-results");
  assert.equal(res.status, 405);
  assert.equal(res.body.accepted, false);
});

test("a token cannot be verified without a datastore, so the write fails loudly", async () => {
  const res = await call("eval-results", {
    method: "POST",
    headers: { Authorization: "Bearer srk_aaaaaaaaaaaaaaaaaaaaaaaa" },
    body: "{}",
  });
  assert.equal(res.status, 503);
  assert.equal(res.body.accepted, false);
  assert.match(res.body.reason, /publishing is disabled/);
});

test("an oversized bundle gets a real 413 rather than a dropped connection", async () => {
  const res = await call("eval-results", { method: "POST", body: "x".repeat(4096) });
  assert.equal(res.status, 413);
  assert.equal(res.body.accepted, false);
  assert.match(res.body.reason, /exceeds 512 bytes/);
});

test("whoami answers honestly for an anonymous caller", async () => {
  const res = await call("auth/whoami");
  assert.equal(res.status, 200);
  assert.deepEqual(res.body, { authenticated: false, kind: "", account_id: "", created_at: "", verified: false });
  assert.equal(res.headers.get("cache-control"), "no-store");
});

test("token issuance without a datastore is 503, never a fake token", async () => {
  const res = await call("auth/tokens", { method: "POST", body: JSON.stringify({ kind: "anonymous" }) });
  assert.equal(res.status, 503);
  assert.equal(res.body.token, undefined);
});

test("github sign-in reports itself as unconfigured instead of failing obscurely", async () => {
  const res = await call("auth/device", { method: "POST" });
  assert.equal(res.status, 503);
  assert.match(res.body.error, /not configured/);
});

test("unknown auth sub-routes 404 without caching", async () => {
  const res = await call("auth/nope");
  assert.equal(res.status, 404);
  assert.equal(res.headers.get("cache-control"), "no-store");
});

test("eval results read back anonymously and stay cacheable", async () => {
  const res = await call("skills/obra/systematic-debugging/eval-results");
  assert.equal(res.status, 200);
  assert.deepEqual(res.body, { slug: "obra/systematic-debugging", cells: [] });
  assert.match(res.headers.get("cache-control"), /s-maxage/);
});

test("eval results for an unknown skill are 404", async () => {
  const res = await call("skills/nobody/nothing/eval-results");
  assert.equal(res.status, 404);
});

test("a token present on a read changes nothing", async () => {
  const anonymous = await call("skills/obra/systematic-debugging");
  const withToken = await call("skills/obra/systematic-debugging", {
    headers: { Authorization: "Bearer srk_aaaaaaaaaaaaaaaaaaaaaaaa" },
  });
  assert.equal(anonymous.status, 200);
  assert.deepEqual(withToken.body, anonymous.body);
});

test("the existing read routes are untouched", async () => {
  const search = await call("skills", { method: "GET" });
  assert.equal(search.status, 200);
  assert.ok(search.body.items.length > 0);

  const suite = await call("eval-suites/debug-basics");
  assert.equal(suite.status, 200);
  assert.equal(suite.body.version, "1");

  const missing = await call("nope");
  assert.equal(missing.status, 404);
});

// The routes hardened after review carry store-backed throttles, and this
// harness deliberately has no store: each one must degrade to its honest
// storeless answer rather than throw on the way to the limiter.

test("subscribe still answers without a datastore, and never claims it stored", async () => {
  const res = await call("subscribe", { method: "POST", body: JSON.stringify({ email: "a@b.co" }) });
  assert.equal(res.status, 200);
  assert.deepEqual(res.body, { ok: false, stored: false });
});

test("token revocation without a datastore is 503, never a fake success", async () => {
  const res = await call("auth/tokens", {
    method: "DELETE",
    headers: { Authorization: "Bearer srk_aaaaaaaaaaaaaaaaaaaaaaaa" },
  });
  assert.equal(res.status, 503);
  assert.match(res.body.reason, /revocation is disabled/);
});

test("maintainer revocation reports the missing store before judging credentials", async () => {
  // Storeless, the answer is the same whether or not a maintainer credential is
  // configured or presented — the route refuses before the compare, so this
  // holds in any environment.
  const res = await call("auth/accounts/gh_nobody/revoke", { method: "POST" });
  assert.equal(res.status, 503);
  assert.match(res.body.reason, /revocation is disabled/);
});

test("an absurdly long search q is capped, not an error", async () => {
  const q = "systematic ".repeat(500);
  const huge = await call("skills", {}, { q });
  const capped = await call("skills", {}, { q: q.slice(0, 128) });
  assert.equal(huge.status, 200);
  assert.ok(Array.isArray(huge.body.items));
  assert.deepEqual(huge.body, capped.body, "everything past the cap must be ignored, not judged");
});
