// Re-tier already-ingested skills without re-pinning them.
//
// The scan tier is derived from CONTENT, and content is deliberately never
// persisted (the registry is index-only and does not rehost). So bumping
// `SCANNER_VERSION` cannot re-score anything from local state — the bytes have
// to come back. This fetches each entry's ALREADY PINNED `raw_content_url`,
// which needs no GitHub token and no repo metadata, verifies the content still
// hashes to what we published, and rewrites `scan_tier` + `scan` in place.
//
// Use this after a rules change. Use `ingest.mjs --refresh` when the SOURCE
// moved (that re-pins to a new commit and re-scans as part of the same pass),
// and `ingest.mjs --rescan` to do both in one go.
//
//   node registry/ingest/rescan.mjs [--limit N] [--concurrency N] [--all] [--dry-run]
//
// Default is incremental: only entries whose stored verdict did not come from
// this scanner over this content are re-fetched. `--all` forces every entry.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { SCANNER_VERSION, scanIsCurrent, scanSkillContent } from "../lib/scan.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, "../..");
const OUT_DIR = path.join(REPO_ROOT, "registry/data");
const API_DIR = path.join(REPO_ROOT, "registry/api");

const args = process.argv.slice(2);
const flag = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 ? parseInt(args[i + 1], 10) : fallback;
};
const LIMIT = flag("--limit", Infinity);
// Modest by default: raw.githubusercontent.com is generous but this walks the
// whole catalog, and a rescan is never latency-critical.
const CONCURRENCY = Math.max(1, Math.min(32, flag("--concurrency", 12)));
const ALL = args.includes("--all");
const DRY_RUN = args.includes("--dry-run");

function contentHash(content) {
  const normalized = content.split("\r\n").join("\n").replace(/\n+$/, "");
  return `sha256:${createHash("sha256").update(normalized, "utf8").digest("hex")}`;
}

const ingested = JSON.parse(readFileSync(path.join(API_DIR, "ingested.json"), "utf8"));
const enriched = JSON.parse(readFileSync(path.join(API_DIR, "enriched.json"), "utf8"));

const todo = ingested.filter((e) => e.raw_content_url && (ALL || !scanIsCurrent(e))).slice(0, LIMIT);
console.log(`scanner v${SCANNER_VERSION}: ${todo.length} of ${ingested.length} entries need a verdict`);

let done = 0;
let fetchFailed = 0;
let hashDrift = 0;
const results = new Map();

async function worker(queue) {
  for (;;) {
    const entry = queue.pop();
    if (!entry) return;
    try {
      const res = await fetch(entry.raw_content_url, { headers: { "user-agent": "skillrank-rescan" } });
      if (!res.ok) {
        fetchFailed += 1;
        continue;
      }
      const body = await res.text();
      const hash = contentHash(body);
      // A pinned commit is immutable, so a mismatch means the published hash is
      // wrong, not that the content moved. Re-tiering off bytes that are not the
      // bytes we publish would attach a verdict to the wrong thing.
      if (entry.content_hash && hash !== entry.content_hash) {
        hashDrift += 1;
        continue;
      }
      const scan = scanSkillContent(body, {
        slug: entry.slug,
        sourceUrl: entry.source_url,
        sourceRepo: entry.source_repo,
      });
      results.set(entry.slug, {
        scan_tier: scan.tier,
        scan: {
          tier: scan.tier,
          score: scan.score,
          scanner_version: scan.scannerVersion,
          content_hash: hash,
          findings: scan.findings.slice(0, 8),
        },
      });
    } catch {
      fetchFailed += 1;
    } finally {
      done += 1;
      if (done % 200 === 0) console.log(`  … ${done}/${todo.length}`);
    }
  }
}

const queue = [...todo].reverse();
await Promise.all(Array.from({ length: CONCURRENCY }, () => worker(queue)));

for (const list of [ingested, enriched]) {
  for (const e of list) {
    const update = results.get(e.slug);
    if (!update) continue;
    e.scan_tier = update.scan_tier;
    e.scan = update.scan;
  }
}

const counts = {};
for (const e of ingested) counts[e.scan_tier || "pending"] = (counts[e.scan_tier || "pending"] || 0) + 1;
const safeShare = ((100 * ((counts.safe || 0) + (counts.low || 0))) / Math.max(1, ingested.length)).toFixed(1);

console.log(
  `re-tiered ${results.size}, ${fetchFailed} unfetchable, ${hashDrift} hash mismatch (left untouched)\n` +
    `distribution: ${Object.entries(counts)
      .sort((a, b) => b[1] - a[1])
      .map(([t, n]) => `${t} ${n}`)
      .join(", ")}\n` +
    `${safeShare}% install without a confirmation prompt`,
);

if (DRY_RUN) {
  console.log("--dry-run: nothing written");
} else {
  const ingestedJson = `${JSON.stringify(ingested, null, 2)}\n`;
  const enrichedJson = `${JSON.stringify(enriched, null, 2)}\n`;
  for (const dir of [OUT_DIR, API_DIR]) {
    writeFileSync(path.join(dir, "ingested.json"), ingestedJson);
    writeFileSync(path.join(dir, "enriched.json"), enrichedJson);
  }
  console.log("wrote registry/{data,api}/{ingested,enriched}.json");
}
