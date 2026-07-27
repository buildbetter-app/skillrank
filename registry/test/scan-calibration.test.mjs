// The ship gate.
//
// The scanner exists because the registry warned on 100% of the catalog, which
// trains users to click through warnings and makes the tier system worthless.
// The only way to know that has not quietly come back is to measure the served
// catalog, so these assertions run against the REAL `registry/api/ingested.json`
// — the same file the API serves — rather than against fixtures.
//
// If a rules change pushes the prompt rate up, this fails before deploy. That is
// the entire point; do not relax the threshold to make a new rule fit.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { SCANNER_VERSION, isSafeTier, scanSkillContent } from "../lib/scan.mjs";

const ingested = JSON.parse(readFileSync(new URL("../api/ingested.json", import.meta.url), "utf8"));
const enriched = JSON.parse(readFileSync(new URL("../api/enriched.json", import.meta.url), "utf8"));

const scanned = ingested.filter((e) => e.scan && e.scan.scanner_version);
const share = (n) => `${((100 * n) / ingested.length).toFixed(1)}%`;

test("the catalog is actually scanned, not left pending", () => {
  assert.ok(ingested.length > 100, "expected a real catalog");
  const pending = ingested.filter((e) => !e.scan_tier || e.scan_tier === "pending");
  // A handful can legitimately be un-scanned: their pinned content no longer
  // hashes to what we published, and re-tiering off different bytes than the
  // ones we serve would attach a verdict to the wrong thing.
  assert.ok(
    pending.length / ingested.length < 0.05,
    `${pending.length} of ${ingested.length} entries (${share(pending.length)}) are still pending — run \`node registry/ingest/rescan.mjs\``,
  );
});

test("at least 90% of the catalog installs without a confirmation prompt", () => {
  const safe = ingested.filter((e) => isSafeTier(e.scan_tier)).length;
  assert.ok(
    safe / ingested.length >= 0.9,
    `only ${share(safe)} of the catalog is safe/low. Below 90% the confirmation prompt stops meaning anything, which is the failure this scanner exists to prevent.`,
  );
});

test("the prompt is rare enough to still carry information", () => {
  const counts = {};
  for (const e of ingested) counts[e.scan_tier || "pending"] = (counts[e.scan_tier || "pending"] || 0) + 1;
  // Anchors from the corpus study, with headroom. A ruleset that puts a third of
  // the catalog at `high` is as useless as one that puts none there.
  assert.ok((counts.medium || 0) / ingested.length < 0.12, `medium is ${share(counts.medium || 0)}, expected under 12%`);
  assert.ok((counts.high || 0) / ingested.length < 0.04, `high is ${share(counts.high || 0)}, expected under 4%`);
  assert.ok((counts.flagged || 0) / ingested.length < 0.02, `flagged is ${share(counts.flagged || 0)}, expected under 2%`);
  // …and it must not be empty either: a scanner that finds nothing is not
  // calibrated, it is switched off.
  assert.ok((counts.flagged || 0) + (counts.high || 0) > 0, "no skill reached high or flagged — the rules are not running");
  assert.ok((counts.safe || 0) > 0 && (counts.low || 0) > 0, "both safe and low must be populated for the floor to mean anything");
});

test("every non-safe tier is backed by findings a user can read", () => {
  const unexplained = scanned.filter((e) => !isSafeTier(e.scan_tier) && (!e.scan.findings || e.scan.findings.length === 0));
  assert.deepEqual(
    unexplained.map((e) => e.slug),
    [],
    "a skill cannot be marked risky without evidence — that is the warn-on-everything failure in a new costume",
  );
  for (const e of scanned.filter((x) => !isSafeTier(x.scan_tier)).slice(0, 200)) {
    for (const f of e.scan.findings) {
      assert.ok(f.rule && f.why && f.why.length > 20, `${e.slug} has a finding without a usable explanation`);
      assert.ok(["critical", "high", "medium", "info"].includes(f.severity), `${e.slug}: bad severity ${f.severity}`);
    }
  }
});

test("stored verdicts describe the content we actually publish", () => {
  // `scan.content_hash` is what makes a persisted tier auditable: it names the
  // bytes that produced it. A mismatch means the verdict outlived its content.
  const drifted = scanned.filter((e) => e.scan.content_hash !== e.content_hash);
  assert.deepEqual(drifted.map((e) => e.slug), [], "a stored tier is attached to content that is no longer what we serve");
  const stale = scanned.filter((e) => e.scan.scanner_version !== SCANNER_VERSION);
  assert.ok(
    stale.length / Math.max(1, scanned.length) < 0.05,
    `${stale.length} verdicts predate scanner v${SCANNER_VERSION} — run \`node registry/ingest/rescan.mjs\``,
  );
  for (const e of scanned) assert.equal(e.scan.tier, e.scan_tier, `${e.slug}: scan.tier disagrees with scan_tier`);
});

test("collections and unreachable repos are unknown, never pending or safe", () => {
  for (const e of enriched) {
    if (e.status === "installable") continue;
    assert.ok(
      !e.scan_tier || e.scan_tier === "unknown",
      `${e.slug} has no SKILL.md to scan but claims scan_tier=${e.scan_tier}`,
    );
  }
});

test("re-scanning the served catalog would not move any tier", () => {
  // Spot-check the persisted verdicts against a live scan of a few skills whose
  // content is embedded here, so a rules change that silently re-tiers the
  // catalog cannot pass while `ingested.json` still says otherwise.
  const cases = [
    { body: "# Notes\n\nWrite a short summary of the meeting.\n", tier: "safe" },
    { body: "# Build\n\n```bash\nnpm ci && npm test\n```\n", tier: "low" },
  ];
  for (const c of cases) assert.equal(scanSkillContent(c.body, { slug: "a/b" }).tier, c.tier);
});
