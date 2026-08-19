// Ingestion pipeline: turn the curated catalog into INSTALL-READY registry
// entries. For each skill we locate its real SKILL.md, pin it to a commit SHA,
// fetch the content, and compute a content hash the SAME way the Rust CLI does
// (skillrank-core::hash) so `skillrank install` hash-verification passes.
//
// Also enriches GitHub heuristics (stars, forks, last push) used for ranking.
//
// Scales to thousands later; for now it attempts every catalog entry and reports
// which are directly installable (single SKILL.md) vs. collections (skip).
//
// Each skill's content is also SCANNED here (registry/lib/scan.mjs) and the
// resulting `ScanTier` stored next to the hash — this is the only point in the
// pipeline where the content exists, since the registry is index-only and never
// persists or rehosts it.
//
//   node registry/ingest/ingest.mjs [--limit N] [--force|--refresh|--rescan]
//
// Output: registry/data/ingested.json  (installable, with content_hash+content)
//         registry/data/enriched.json  (all attempts, with signals + status)

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

import { SCANNER_VERSION, scanIsCurrent, scanSkillContent } from "../lib/scan.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, "../..");
const catalog = JSON.parse(readFileSync(path.join(REPO_ROOT, "web/data/catalog.json"), "utf8"));

const args = process.argv.slice(2);
const limitFlag = args.indexOf("--limit");
const LIMIT = limitFlag >= 0 ? parseInt(args[limitFlag + 1], 10) : Infinity;
const FORCE = args.includes("--force"); // re-fetch everything; default is incremental
const REFRESH = args.includes("--refresh"); // re-pin only skills whose source repo advanced
// Re-fetch and re-tier anything whose stored verdict came from an older
// scanner. The tier is derived from CONTENT, and content is deliberately not
// persisted (index-only, we never rehost), so a rules bump means a re-fetch —
// there is nothing local to re-score. Without this, the incremental branch below
// would freeze every tier at whatever the scanner said the day it was pinned.
const RESCAN = args.includes("--rescan");

// Incremental: reuse already-ingested skills (by slug) so CI only fetches new
// ones. `--force` re-does the whole catalog.
const OUT_DIR = path.join(REPO_ROOT, "registry/data");
const API_DIR = path.join(REPO_ROOT, "registry/api");
function loadPrev(file) {
  try {
    return new Map(JSON.parse(readFileSync(path.join(OUT_DIR, file), "utf8")).map((e) => [e.slug, e]));
  } catch {
    return new Map();
  }
}
const prevIngested = FORCE ? new Map() : loadPrev("ingested.json");
const prevEnriched = FORCE ? new Map() : loadPrev("enriched.json");

// --- content hash, matching skillrank-core::hash::compute_content_hash ---
function contentHash(content) {
  const normalized = content.split("\r\n").join("\n").replace(/\n+$/, "");
  return "sha256:" + createHash("sha256").update(normalized, "utf8").digest("hex");
}

// --- thin gh helpers (authenticated in this env) ---
function gh(pathAndQuery) {
  try {
    const out = execFileSync("gh", ["api", pathAndQuery], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
    return JSON.parse(out);
  } catch {
    return null;
  }
}
function ghRaw(repo, sha, filePath) {
  // fetch a blob's raw text at a pinned commit
  const res = gh(`repos/${repo}/contents/${encodeURI(filePath)}?ref=${sha}`);
  if (!res || !res.content) return null;
  return Buffer.from(res.content, res.encoding === "base64" ? "base64" : "utf8").toString("utf8");
}

function rawUrl(repo, sha, filePath) {
  const encodedPath = filePath.split("/").map(encodeURIComponent).join("/");
  return `https://raw.githubusercontent.com/${repo}/${sha}/${encodedPath}`;
}

function rawBytes(repo, sha, filePath) {
  try {
    return execFileSync("curl", ["-fsSL", "--max-time", "20", rawUrl(repo, sha, filePath)], {
      encoding: "buffer",
      stdio: ["ignore", "pipe", "ignore"],
      maxBuffer: 6 * 1024 * 1024,
    });
  } catch {
    return null;
  }
}

// per-repo cache: metadata + head commit sha
const repoCache = new Map();
function repoInfo(repo) {
  if (repoCache.has(repo)) return repoCache.get(repo);
  const meta = gh(`repos/${repo}`);
  let info = null;
  if (meta && meta.default_branch) {
    const branch = meta.default_branch;
    const commit = gh(`repos/${repo}/commits/${branch}`);
    const headSha = commit && commit.sha ? commit.sha : null;
    const license = headSha ? gh(`repos/${repo}/license?ref=${headSha}`) : null;
    const tree = headSha ? gh(`repos/${repo}/git/trees/${headSha}?recursive=1`) : null;
    const spdx = meta.license?.spdx_id;
    info = {
      default_branch: branch,
      head_sha: headSha,
      stars: meta.stargazers_count ?? null,
      forks: meta.forks_count ?? null,
      pushed_at: meta.pushed_at ?? null,
      open_issues: meta.open_issues_count ?? null,
      license_spdx: spdx && spdx !== "NOASSERTION" && spdx !== "other" ? spdx : "",
      license_url: typeof license?.html_url === "string" ? license.html_url : "",
      tree: Array.isArray(tree?.tree) && tree.truncated !== true ? tree.tree : [],
      tree_complete: Array.isArray(tree?.tree) && tree.truncated !== true,
    };
  }
  repoCache.set(repo, info);
  return info;
}

const MAX_PACKAGE_ASSETS = 50;
const MAX_PACKAGE_ASSET_BYTES = 1024 * 1024;
const MAX_PACKAGE_BYTES = 5 * 1024 * 1024;

function packageManifest(repo, info, skillPath) {
  if (!info.tree_complete) {
    return { kind: "unsupported", manifest_version: 1, assets: [], reason: "source tree could not be verified" };
  }
  const directory = path.posix.dirname(skillPath);
  const prefix = directory === "." ? "" : `${directory}/`;
  const candidates = info.tree
    .filter((entry) => entry.type === "blob" && entry.path !== skillPath && (!prefix || entry.path.startsWith(prefix)))
    .sort((a, b) => a.path.localeCompare(b.path));
  if (candidates.length === 0) return { kind: "self_contained", manifest_version: 1, assets: [] };
  if (candidates.length > MAX_PACKAGE_ASSETS) {
    return { kind: "unsupported", manifest_version: 1, assets: [], reason: `package has more than ${MAX_PACKAGE_ASSETS} companion files` };
  }
  const totalBytes = candidates.reduce((sum, entry) => sum + (Number.isFinite(entry.size) ? entry.size : MAX_PACKAGE_BYTES + 1), 0);
  if (candidates.some((entry) => !Number.isFinite(entry.size) || entry.size > MAX_PACKAGE_ASSET_BYTES) || totalBytes > MAX_PACKAGE_BYTES) {
    return { kind: "unsupported", manifest_version: 1, assets: [], reason: "package exceeds verified asset limits" };
  }
  const assets = [];
  for (const entry of candidates) {
    const relative = path.posix.relative(directory === "." ? "" : directory, entry.path);
    if (!relative || relative.startsWith("../") || path.posix.isAbsolute(relative)) {
      return { kind: "unsupported", manifest_version: 1, assets: [], reason: "package contains an unsafe asset path" };
    }
    const bytes = rawBytes(repo, info.head_sha, entry.path);
    if (!bytes || bytes.length !== entry.size) {
      return { kind: "unsupported", manifest_version: 1, assets: [], reason: `could not verify companion file ${relative}` };
    }
    assets.push({
      path: relative,
      byte_size: bytes.length,
      content_hash: "sha256:" + createHash("sha256").update(bytes).digest("hex"),
      raw_content_url: rawUrl(repo, info.head_sha, entry.path),
    });
  }
  return { kind: "bundle", manifest_version: 1, assets };
}

function candidatePaths(skill) {
  const sub = (skill.source_subpath || "").replace(/\/+$/, "");
  if (sub) return [`${sub}/SKILL.md`, `${sub}/skill.md`];
  return ["SKILL.md", "skill.md"];
}

// --- provisional score with a slightly richer heuristic (stars + forks) ---
const maxStars = Math.max(...catalog.map((s) => s.signals?.stars || 0));
const denom = Math.log10(maxStars + 1);
function provisionalScore(stars, forks) {
  const c = Math.log10((stars || 0) + 1) / denom; // community, log-scaled 0..1
  const f = Math.min(1, Math.log10((forks || 0) + 1) / Math.log10(20000)); // activity proxy
  const blended = 0.85 * c + 0.15 * f;
  return Math.round(Math.min(75, 74 * blended));
}

const ordered = [...catalog].sort((a, b) => (b.signals?.stars || 0) - (a.signals?.stars || 0)).slice(0, LIMIT);

const installable = [];
const enriched = [];
/// Cap what lands in `ingested.json`. The tier is decided by the worst few
/// findings, and a confirmation dialog cannot use twenty; this keeps the served
/// JSON from growing by megabytes for skills that repeat one pattern.
const MAX_PERSISTED_FINDINGS = 8;
const tierCounts = {};
let ok = 0,
  collection = 0,
  failed = 0;

let reused = 0;
let repinned = 0;
let rescanned = 0;
let metadataBackfilled = 0;
for (const skill of ordered) {
  // incremental: keep prior result, skip network.
  if (prevEnriched.has(skill.slug)) {
    const prev = prevIngested.get(skill.slug);
    const previousEntry = prevEnriched.get(skill.slug);
    const metadataCurrent =
      Object.hasOwn(previousEntry, "license_spdx") &&
      Object.hasOwn(previousEntry, "license_url") &&
      (!prev || (prev.package?.manifest_version === 1 && Array.isArray(prev.package.assets)));
    const staleScan = RESCAN && prev && !scanIsCurrent(prev);
    // --refresh: re-ingest installable skills whose source repo advanced past
    // the commit we pinned; otherwise reuse (cheap: one HEAD check per repo).
    if (REFRESH && prev && !staleScan && metadataCurrent) {
      const info = repoInfo(skill.source_repo);
      if (info && info.head_sha && prev.pinned_commit && info.head_sha === prev.pinned_commit) {
        enriched.push(prevEnriched.get(skill.slug));
        installable.push(prev);
        reused++;
        continue;
      }
      repinned++; // source advanced → fall through and re-fetch/re-pin
    } else if (staleScan) {
      rescanned++; // verdict predates the current rules → fall through and re-tier
    } else if (metadataCurrent) {
      enriched.push(previousEntry);
      if (prev) installable.push(prev);
      reused++;
      continue;
    } else {
      metadataBackfilled++;
    }
  }
  const info = repoInfo(skill.source_repo);
  const base = {
    slug: skill.slug,
    display_name: skill.display_name,
    category: skill.category,
    tags: skill.tags,
    source_type: "github",
    source_url: skill.source_url,
    source_repo: skill.source_repo,
    source_subpath: skill.source_subpath,
    description: skill.description,
    tier: skill.eval.tier,
    agents: Array.isArray(skill.agents) ? skill.agents : [],
  };

  if (!info || !info.head_sha) {
    failed++;
    enriched.push({ ...base, status: "repo_unreachable", signals: { stars: skill.signals.stars, forks: null } });
    console.log(`  ✗ ${skill.slug} — repo unreachable`);
    continue;
  }

  const signals = { stars: info.stars, forks: info.forks, installs: null, pushed_at: info.pushed_at };
  const score = provisionalScore(info.stars, info.forks);
  const importMetadata = { license_spdx: info.license_spdx, license_url: info.license_url };

  let found = null;
  for (const p of candidatePaths(skill)) {
    const content = ghRaw(skill.source_repo, info.head_sha, p);
    if (content && content.trim().length > 0) {
      found = { path: p, content };
      break;
    }
  }

  if (!found) {
    collection++;
    enriched.push({
      ...base,
      ...importMetadata,
      package: { kind: "unsupported", manifest_version: 1, assets: [], reason: "source has no single SKILL.md" },
      status: "collection",
      signals,
      score,
    });
    console.log(`  ~ ${skill.slug} — no single SKILL.md (collection/other layout)`);
    continue;
  }

  const hash = contentHash(found.content);
  const rawUrl = `https://raw.githubusercontent.com/${skill.source_repo}/${info.head_sha}/${found.path}`;
  // The content is in hand exactly here and nowhere else — it is deliberately
  // not persisted — so the tier has to be computed now and stored as a scalar
  // next to the hash it belongs to.
  const scan = scanSkillContent(found.content, {
    slug: skill.slug,
    sourceUrl: skill.source_url,
    sourceRepo: skill.source_repo,
  });
  tierCounts[scan.tier] = (tierCounts[scan.tier] || 0) + 1;
  ok++;
  const entry = {
    ...base,
    ...importMetadata,
    pinned_commit: info.head_sha,
    skill_path: found.path,
    content_hash: hash,
    raw_content_url: rawUrl,
    scan_tier: scan.tier,
    // `content_hash` inside the report is what makes the verdict auditable: it
    // says WHICH bytes were scanned, so a tier can never silently outlive the
    // content it describes.
    scan: {
      tier: scan.tier,
      score: scan.score,
      scanner_version: scan.scannerVersion,
      content_hash: hash,
      findings: scan.findings.slice(0, MAX_PERSISTED_FINDINGS),
    },
    package: packageManifest(skill.source_repo, info, found.path),
    signals,
    score,
    provisional: true,
    installable: true,
  };
  // index-only: resolve serves raw_content_url + hash, never rehosts content.
  installable.push(entry);
  enriched.push({ ...entry, status: "installable" });
  const worst = scan.findings[0];
  console.log(
    `  ✓ ${skill.slug} — ${found.path} @ ${info.head_sha.slice(0, 7)} (${hash.slice(0, 14)}…) [${scan.tier}${worst ? ` · ${worst.rule}` : ""}]`,
  );
}

mkdirSync(OUT_DIR, { recursive: true });
mkdirSync(API_DIR, { recursive: true });
const ingestedJson = JSON.stringify(installable, null, 2) + "\n";
const enrichedJson = JSON.stringify(enriched, null, 2) + "\n";
for (const dir of [OUT_DIR, API_DIR]) {
  writeFileSync(path.join(dir, "ingested.json"), ingestedJson);
  writeFileSync(path.join(dir, "enriched.json"), enrichedJson);
}

console.log(
  `\nattempted ${ordered.length}: ${reused} reused, ${metadataBackfilled} import-metadata backfills, ${repinned} re-pinned, ${rescanned} re-scanned, ${ok} newly ingested, ${collection} collections, ${failed} failed`
);
console.log(`wrote registry/{data,api}/ingested.json (${installable.length}) + enriched.json (${enriched.length})`);

// Scan-tier distribution over EVERY installable entry, reused ones included.
// The whole point of the scanner is that legitimate skills come back safe/low,
// so print the ratio that proves it (or does not) on every run.
const overall = {};
for (const e of installable) overall[e.scan_tier || "pending"] = (overall[e.scan_tier || "pending"] || 0) + 1;
const safeShare = ((100 * ((overall.safe || 0) + (overall.low || 0))) / Math.max(1, installable.length)).toFixed(1);
console.log(
  `scan v${SCANNER_VERSION}: ${Object.entries(overall)
    .sort((a, b) => b[1] - a[1])
    .map(([t, n]) => `${t} ${n}`)
    .join(", ")} — ${safeShare}% install without a confirmation prompt` +
    (Object.keys(tierCounts).length ? ` (${ok} scanned this run)` : ""),
);
