import assert from "node:assert/strict";
import { createServer } from "node:http";
import test, { after, before } from "node:test";

for (const key of ["KV_REST_API_URL", "KV_REST_API_TOKEN", "UPSTASH_REDIS_REST_URL", "UPSTASH_REDIS_REST_TOKEN"]) {
  delete process.env[key];
}

const { default: handler } = await import("../api/registry.mjs");

let server;
let base;

before(async () => {
  server = createServer((req, res) => handler(req, res));
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  base = `http://127.0.0.1:${server.address().port}`;
});

after(() => server.close());

async function call(path, query = {}) {
  const res = await fetch(`${base}/api/registry?${new URLSearchParams({ path, ...query })}`);
  return { status: res.status, headers: res.headers, body: await res.json().catch(() => null) };
}

test("search caps limits and traverses stable cursor pages without duplicates", async () => {
  const first = await call("skills", { limit: "10000", sort: "name" });
  assert.equal(first.status, 200);
  assert.equal(first.body.items.length, 100);
  assert.equal(typeof first.body.next_cursor, "string");

  const slugs = new Set();
  let cursor = "";
  let total = null;
  do {
    const page = await call("skills", { limit: "97", sort: "name", cursor });
    assert.equal(page.status, 200);
    total ??= page.body.total;
    assert.equal(page.body.total, total);
    for (const item of page.body.items) {
      assert.equal(slugs.has(item.slug), false, `duplicate ${item.slug}`);
      slugs.add(item.slug);
    }
    cursor = page.body.next_cursor || "";
  } while (cursor);

  assert.equal(slugs.size, total);
});

test("a cursor is bound to its filters and sort", async () => {
  const first = await call("skills", { limit: "2", category: "testing", sort: "signals" });
  assert.ok(first.body.next_cursor);
  const changed = await call("skills", { limit: "2", category: "meta", sort: "signals", cursor: first.body.next_cursor });
  assert.equal(changed.status, 400);
  assert.equal(changed.body.code, "invalid_cursor");
});

test("search exposes import policy metadata on every row", async () => {
  const res = await call("skills", { limit: "20", agent: "codex", scan_tier: "safe", sort: "signals" });
  assert.equal(res.status, 200);
  for (const item of res.body.items) {
    for (const field of ["license_spdx", "license_url", "package_kind", "collection_ids", "agents"]) {
      assert.ok(field in item, `${item.slug} missing ${field}`);
    }
    assert.equal(item.scan_tier, "safe");
    assert.ok(item.agents.length === 0 || item.agents.includes("codex"));
  }
});

test("resolve verifies the requested version and returns license and package evidence", async () => {
  const search = await call("skills", { limit: "1", scan_tier: "safe" });
  const skill = search.body.items[0];
  const resolved = await call(`skills/${skill.slug}/resolve`, { version: skill.latest_version });
  assert.equal(resolved.status, 200);
  assert.equal(resolved.body.content_hash, skill.latest_version);
  assert.ok("license_spdx" in resolved.body);
  assert.ok("license_url" in resolved.body);
  assert.ok(["self_contained", "bundle", "unsupported", "unknown"].includes(resolved.body.package.kind));
  assert.equal(resolved.body.package.manifest_version, 1);
  assert.ok(Array.isArray(resolved.body.package.assets));

  const changed = await call(`skills/${skill.slug}/resolve`, { version: "sha256:0000000000000000000000000000000000000000000000000000000000000000" });
  assert.equal(changed.status, 409);
  assert.equal(changed.body.code, "registry_changed");
  assert.equal(changed.body.current_content_hash, skill.latest_version);
});

test("collections are first-class, cursor-paginated repository groups", async () => {
  const list = await call("collections", { limit: "5" });
  assert.equal(list.status, 200);
  assert.ok(list.body.items.length > 0);
  assert.ok(list.body.items.every((item) => item.id.includes("/")));

  const collection = list.body.items.find((item) => item.member_count > 0) || list.body.items[0];
  const detail = await call(`collections/${collection.id}`, { limit: "3" });
  assert.equal(detail.status, 200);
  assert.equal(detail.body.id, collection.id);
  assert.equal(typeof detail.body.collection_hash, "string");
  assert.ok(Array.isArray(detail.body.members));
  assert.ok(Array.isArray(detail.body.excluded));
  assert.ok(detail.body.members.every((member) => member.slug && member.content_hash));
});
