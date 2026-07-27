// The API half of the fix.
//
// Computing a tier during ingest changes nothing on its own: `registry.mjs` used
// to discard `e.scan_tier` and answer `pending` for everything installable and
// `unknown` for everything else, in five separate places. These tests drive the
// real deployed function over HTTP and assert that what the pipeline stored is
// what the client receives — including that the new `scan` object is strictly
// ADDITIVE, since a compiled Rust `SkillDetail` / `ResolveResponse` has to keep
// deserializing responses from a registry that ships before the client does.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import test, { after, before } from "node:test";

import { isSafeTier } from "../lib/scan.mjs";

for (const key of ["KV_REST_API_URL", "KV_REST_API_TOKEN", "UPSTASH_REDIS_REST_URL", "UPSTASH_REDIS_REST_TOKEN"]) {
  delete process.env[key];
}

const { default: handler } = await import("../api/registry.mjs");
const ingested = JSON.parse(readFileSync(new URL("../api/ingested.json", import.meta.url), "utf8"));
const enriched = JSON.parse(readFileSync(new URL("../api/enriched.json", import.meta.url), "utf8"));

let server;
let base;

before(async () => {
  server = createServer((req, res) => handler(req, res));
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  base = `http://127.0.0.1:${server.address().port}`;
});
after(() => server.close());

/// Mirrors `registry/vercel.json`: /v3/rest/skill-registry/:path* -> ?path=:path*
/// with the caller's own query string preserved alongside it.
async function call(path, init = {}, query = {}) {
  const res = await fetch(`${base}/api/registry?${new URLSearchParams({ path, ...query })}`, init);
  return { status: res.status, headers: res.headers, body: await res.json().catch(() => null) };
}

const TIERS = new Set(["safe", "low", "medium", "high", "flagged", "pending", "unknown"]);
const sample = (pred) => ingested.find(pred);

test("search serves the tier the pipeline computed, not a hardcoded pending", () => {
  return call("skills", {}, { limit: "50" }).then((res) => {
    assert.equal(res.status, 200);
    assert.ok(res.body.items.length > 0);
    for (const item of res.body.items) assert.ok(TIERS.has(item.scan_tier), `bad tier ${item.scan_tier}`);
    const pending = res.body.items.filter((i) => i.scan_tier === "pending");
    assert.ok(
      pending.length < res.body.items.length / 2,
      "most of the catalog is still answering 'pending' — the API is discarding the stored tier again",
    );
  });
});

test("the facets expose real tiers so a client can build a filter that matches something", async () => {
  const res = await call("skills/facets");
  assert.equal(res.status, 200);
  const values = res.body.scan_tiers.map((f) => f.value);
  for (const v of values) assert.ok(TIERS.has(v), `facet offers unknown tier ${v}`);
  assert.ok(values.includes("safe") || values.includes("low"), "no safe/low facet — nothing is being scanned");
  // Every count must be the `total` the corresponding filter returns.
  const total = res.body.scan_tiers.reduce((n, f) => n + f.count, 0);
  assert.equal(total, enriched.length);
});

test("detail carries the tier and the findings behind it", async () => {
  const risky = sample((e) => e.scan && !isSafeTier(e.scan_tier) && e.scan.findings.length > 0);
  assert.ok(risky, "expected at least one non-safe skill in the catalog");
  const res = await call(`skills/${risky.slug}`);
  assert.equal(res.status, 200);
  assert.equal(res.body.scan_tier, risky.scan_tier);
  assert.equal(res.body.versions[0].scan_tier, risky.scan_tier);
  assert.equal(res.body.scan.tier, risky.scan_tier);
  assert.equal(res.body.scan.scanner_version, risky.scan.scanner_version);
  assert.ok(res.body.scan.findings.length > 0);
  const f = res.body.scan.findings[0];
  assert.deepEqual(Object.keys(f).sort(), ["excerpt", "line", "rule", "severity", "why"]);
  assert.ok(f.why.length > 20, "a user seeing 'medium risk' must be able to learn exactly what caused it");
});

test("resolve carries the tier and findings, because install is where it matters", async () => {
  const risky = sample((e) => e.scan && !isSafeTier(e.scan_tier) && e.scan.findings.length > 0);
  const res = await call(`skills/${risky.slug}/resolve`);
  assert.equal(res.status, 200);
  assert.equal(res.body.scan_tier, risky.scan_tier);
  assert.equal(res.body.content_hash, risky.content_hash);
  assert.equal(res.body.scan.tier, risky.scan_tier);
  assert.ok(res.body.scan.findings.length > 0);
});

test("a safe skill resolves without a prompt and without a findings payload", async () => {
  const ok = sample((e) => e.scan_tier === "safe");
  assert.ok(ok, "expected at least one safe skill");
  const res = await call(`skills/${ok.slug}/resolve`);
  assert.equal(res.status, 200);
  assert.equal(res.body.scan_tier, "safe");
  assert.equal(isSafeTier(res.body.scan_tier), true);
  assert.equal(res.body.scan.findings.length, 0, "a safe response should not carry an empty-risk narrative");
});

test("collections still resolve to an unknown tombstone", async () => {
  const collection = enriched.find((e) => e.status !== "installable");
  if (!collection) return;
  const res = await call(`skills/${collection.slug}/resolve`);
  assert.equal(res.status, 200);
  assert.equal(res.body.tombstoned, true);
  assert.equal(res.body.scan_tier, "unknown", "no SKILL.md exists, so there is genuinely no verdict");
  assert.equal(res.body.scan, undefined);
});

test("the scan object is additive — every pre-existing field is unchanged", async () => {
  const entry = sample((e) => e.scan);
  const detail = await call(`skills/${entry.slug}`);
  const resolve = await call(`skills/${entry.slug}/resolve`);
  // Exactly the fields the compiled Rust types expect, plus `scan`.
  for (const k of ["slug", "display_name", "category", "stacks", "source_type", "source_url", "latest_version", "scan_tier", "signals_score", "rating_count", "summary", "versions", "eval_cells"]) {
    assert.ok(k in detail.body, `detail lost field ${k}`);
  }
  for (const k of ["slug", "version", "source_type", "source_url", "source_subpath", "pinned_commit", "content_hash", "scan_tier", "signals_score", "raw_content_url", "tombstoned"]) {
    assert.ok(k in resolve.body, `resolve lost field ${k}`);
  }
  assert.equal(typeof detail.body.scan, "object");
  assert.equal(typeof resolve.body.scan, "object");
});

test("an unrecognized stored tier degrades to unknown instead of breaking the client", async () => {
  // `scan_tier` deserializes into a Rust enum. A value outside the seven
  // variants fails the WHOLE response, so it must never reach the wire.
  const { normalizeTier } = await import("../lib/scan.mjs");
  assert.equal(normalizeTier("catastrophic"), "unknown");
  assert.equal(normalizeTier("Safe"), "safe");
  const res = await call("skills", {}, { limit: "200" });
  for (const item of res.body.items) assert.ok(TIERS.has(item.scan_tier));
});

test("reads stay anonymous and cacheable with the scan attached", async () => {
  const entry = sample((e) => e.scan);
  const anonymous = await call(`skills/${entry.slug}`);
  const withToken = await call(`skills/${entry.slug}`, {
    headers: { Authorization: "Bearer srk_aaaaaaaaaaaaaaaaaaaaaaaa" },
  });
  assert.deepEqual(withToken.body, anonymous.body);
  assert.match(anonymous.headers.get("cache-control"), /s-maxage/);
});
