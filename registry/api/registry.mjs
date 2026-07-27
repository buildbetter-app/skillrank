// Hosted SkillRank registry (read side) — serves the /v3/rest/skill-registry
// contract from the ingested public-skill catalog.
//
//   search / show  -> over ALL catalog entries (enriched.json)
//   resolve        -> installable entries carry pinned commit + content + hash
//                     (ingested.json); collections resolve to a tombstone
//                     pointing at the source repo.
//
// Every route here is anonymous and edge-cacheable, and answers identically
// whether or not a token is presented.
//
// Content hashes were computed by the ingestion pipeline the SAME way as the Rust
// client (skillrank-core::hash), so `skillrank install` hash-verification passes.

import { readFileSync } from "node:fs";
import { Redis } from "@upstash/redis";

import { normalizeTier } from "../lib/scan.mjs";

const enriched = JSON.parse(readFileSync(new URL("./enriched.json", import.meta.url), "utf8"));
const ingested = JSON.parse(readFileSync(new URL("./ingested.json", import.meta.url), "utf8"));
// NOTE: use LITERAL new URL(...) args (not a variable path) so Vercel's file
// tracer bundles these JSON files into the deployed function.
function safeRead(url, fallback) {
  try {
    return JSON.parse(readFileSync(url, "utf8"));
  } catch {
    return fallback;
  }
}
const suites = safeRead(new URL("./suites.json", import.meta.url), []);
const verifiers = safeRead(new URL("./verifiers.json", import.meta.url), {});

const bySlug = new Map(enriched.map((e) => [e.slug, e]));
const installBySlug = new Map(ingested.map((e) => [e.slug, e]));
const sorted = [...enriched].sort((a, b) => (b.score || 0) - (a.score || 0));

/// The tier the ingestion pipeline computed from the pinned content.
///
/// This used to return a hardcoded `"pending"` for everything installable, which
/// meant nothing the pipeline wrote was ever served: 188 of 200 skills came back
/// `pending` and 12 `unknown`, so the client treated 100% of the catalog as
/// unverified and prompted on every install.
///
/// Entries with no `scan_tier` are ones whose content was never fetched
/// (collections, unreachable repos) or that were pinned before the scanner
/// existed — `unknown` and `pending` respectively, which is the truth in both
/// cases. The value is normalized because `scan_tier` deserializes into a Rust
/// enum on the client: an unrecognized string would fail the whole response, so
/// a bad value degrades to `unknown` instead of breaking the caller.
function scanTier(e) {
  if (!e) return "unknown";
  // `enriched.json` marks this with `status`; `ingested.json` rows carry
  // `installable` instead, and both flow through here.
  const fallback = e.status === "installable" || e.installable === true ? "pending" : "unknown";
  if (typeof e.scan_tier === "string" && e.scan_tier) return normalizeTier(e.scan_tier, fallback);
  return fallback;
}

/// Findings behind a non-safe tier, shaped for a confirmation dialog.
///
/// ADDITIVE ONLY. The compiled Rust `SkillDetail` / `ResolveResponse` must keep
/// deserializing, so this is a new optional object under a new key and no
/// existing field changes shape. Omitted entirely when there is nothing to say,
/// so `safe` responses do not grow.
function scanReport(e) {
  const scan = e && e.scan;
  if (!scan || typeof scan !== "object") return null;
  const findings = Array.isArray(scan.findings) ? scan.findings : [];
  return {
    tier: scanTier(e),
    score: typeof scan.score === "number" ? scan.score : 0,
    scanner_version: typeof scan.scanner_version === "string" ? scan.scanner_version : "",
    findings: findings.slice(0, 16).map((f) => ({
      rule: String(f.rule || ""),
      severity: String(f.severity || ""),
      line: Number.isFinite(f.line) ? f.line : 0,
      excerpt: String(f.excerpt || ""),
      why: String(f.why || ""),
    })),
  };
}

function summary(e) {
  return {
    slug: e.slug,
    display_name: e.display_name,
    category: e.category || "",
    stacks: e.tags || [],
    source_type: "github",
    source_url: e.source_url || "",
    latest_version: e.content_hash || "",
    scan_tier: scanTier(e),
    signals_score: typeof e.score === "number" ? e.score : null,
    rating_count: 0,
    summary: e.description || "",
  };
}

/// One row of the eval-suite index. Task INSTRUCTIONS are the bulk of a suite and
/// a caller choosing between suites cannot use them, so the list carries only what
/// a picker needs — id, version, size, and the reference env a run must match —
/// and `GET /eval-suites/:id` still serves the full definition.
function suiteSummary(s) {
  const out = {
    id: s.id || "",
    version: s.version || "",
    task_count: Array.isArray(s.tasks) ? s.tasks.length : 0,
  };
  // Omitted rather than blanked when a suite has no title/description, so a
  // client can tell "not provided" from "provided and empty".
  if (s.title) out.title = String(s.title);
  if (s.description) out.description = String(s.description);
  if (s.reference_env) out.reference_env = s.reference_env;
  return out;
}

/// Count how many catalog entries carry each value of one facet.
///
/// Values are normalized exactly the way the `/skills` filters compare them
/// (trimmed, lowercased) and de-duplicated per entry, so a `count` here is
/// precisely the `total` that filtering on that value returns — a client that
/// renders these as filter options can never offer one that matches nothing.
/// Ties break alphabetically so the order is stable across deploys.
function countFacet(pick) {
  const counts = new Map();
  for (const e of enriched) {
    const seen = new Set();
    for (const raw of pick(e)) {
      const value = String(raw ?? "").trim().toLowerCase();
      if (!value || seen.has(value)) continue;
      seen.add(value);
      counts.set(value, (counts.get(value) || 0) + 1);
    }
  }
  return [...counts.entries()]
    .map(([value, count]) => ({ value, count }))
    .sort((a, b) => b.count - a.count || (a.value < b.value ? -1 : 1));
}

// The catalog is a static import, so the taxonomy is a constant: compute it once
// at module load and let the route just serialize it.
const facets = {
  categories: countFacet((e) => [e.category]),
  stacks: countFacet((e) => e.tags || []),
  scan_tiers: countFacet((e) => [scanTier(e)]),
};

const stripSep = (s) => s.replace(/[\s\-_/]/g, "");
function matchesQuery(e, q) {
  const hay = [e.slug, e.display_name, e.summary || e.description, e.category, (e.tags || []).join(" "), e.source_repo]
    .join(" ")
    .toLowerCase();
  const collapsed = stripSep(q);
  if (collapsed && stripSep(hay).includes(collapsed)) return true;
  const words = q.split(/\s+/).filter(Boolean);
  return words.length > 0 && words.every((w) => hay.includes(w));
}

function json(res, status, body, cache = "public, s-maxage=60, stale-while-revalidate=600") {
  res.statusCode = status;
  res.setHeader("Content-Type", "application/json; charset=utf-8");
  res.setHeader("Cache-Control", cache);
  res.end(JSON.stringify(body));
}

// ---- optional datastore (Upstash Redis) ---------------------------------
// Everything below degrades to a NO-OP when the store is not provisioned, so
// the read-side registry keeps working with zero env vars. We only ever store
// aggregate counters and emails the user explicitly typed — never IPs or PII.
let _redis;
let _redisInit = false;
function redis() {
  if (_redisInit) return _redis;
  _redisInit = true;
  try {
    const url = process.env.KV_REST_API_URL || process.env.UPSTASH_REDIS_REST_URL;
    const token = process.env.KV_REST_API_TOKEN || process.env.UPSTASH_REDIS_REST_TOKEN;
    if (url && token) _redis = new Redis({ url, token });
  } catch {
    _redis = undefined;
  }
  return _redis;
}

async function bumpInstall(slug) {
  const r = redis();
  if (!r) return;
  try {
    await r.zincrby("installs", 1, slug);
  } catch {
    /* telemetry must never break a resolve */
  }
}

function readBody(req) {
  return new Promise((resolve) => {
    let data = "";
    req.on("data", (c) => {
      data += c;
      if (data.length > 8192) req.destroy(); // cap body size
    });
    req.on("end", () => resolve(data));
    req.on("error", () => resolve(""));
  });
}

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

export default async function handler(req, res) {
  const url = new URL(req.url, "http://x");
  const path = url.searchParams.get("path") || "";
  const parts = path.split("/").filter(Boolean);

  // ---- email capture: POST /subscribe { email } ----
  if (parts[0] === "subscribe") {
    if (req.method !== "POST") return json(res, 405, { error: "method not allowed" }, "no-store");
    const raw = await readBody(req);
    let email = "";
    try {
      email = String(JSON.parse(raw || "{}").email || "").trim().toLowerCase();
    } catch {
      email = "";
    }
    if (!EMAIL_RE.test(email) || email.length > 254) {
      return json(res, 400, { ok: false, error: "invalid email" }, "no-store");
    }
    const r = redis();
    let stored = false;
    if (r) {
      try {
        await r.sadd("emails", email);
        stored = true;
      } catch {
        stored = false;
      }
    }
    return json(res, 200, { ok: stored, stored }, "no-store");
  }

  // ---- popularity: GET /installs?limit=N -> top slugs by install-intent ----
  if (parts[0] === "installs") {
    const limit = Math.max(1, Math.min(100, parseInt(url.searchParams.get("limit") || "20", 10) || 20));
    const r = redis();
    let items = [];
    if (r) {
      try {
        const raw = await r.zrange("installs", 0, limit - 1, { rev: true, withScores: true });
        for (let i = 0; i < raw.length; i += 2) items.push({ slug: raw[i], count: Number(raw[i + 1]) });
      } catch {
        items = [];
      }
    }
    return json(res, 200, { items }, "public, s-maxage=30");
  }

  // ---- eval suites: /eval-suites, /eval-suites/:id, /eval-suites/:id/verifiers ----
  if (parts[0] === "eval-suites") {
    const id = parts[1] || "";
    // The index. Without it a suite id can only be typed from memory, which is
    // not a discoverable contract; it is a plain anonymous read like its
    // neighbours, and deliberately omits task bodies.
    if (!id) return json(res, 200, { items: suites.map(suiteSummary), total: suites.length });
    const suite = suites.find((s) => s.id === id);
    if (!suite) return json(res, 404, { error: "suite not found" });
    if (parts[2] === "verifiers") {
      return json(res, 200, verifiers[id] || {});
    }
    return json(res, 200, suite);
  }

  if (parts[0] !== "skills") return json(res, 404, { error: "not found" });
  const rest = parts.slice(1);

  // /skills -> search
  if (rest.length === 0) {
    const q = (url.searchParams.get("q") || "").toLowerCase();
    const stack = (url.searchParams.get("stack") || "").toLowerCase();
    const category = (url.searchParams.get("category") || "").toLowerCase();
    // `/skills/facets` advertises scan tiers as a filter vocabulary, so this has
    // to honour them: without it, picking a tier facet silently returned the
    // whole catalog instead of the count the facet promised. Compared through
    // `scanTier` rather than the raw field so the same normalization (and the
    // pending/unknown fallback) applies on both sides.
    const scan = (url.searchParams.get("scan_tier") || "").toLowerCase();
    const limit = Math.max(1, parseInt(url.searchParams.get("limit") || "20", 10) || 20);
    let items = sorted.filter((e) => {
      if (stack && !(e.tags || []).some((s) => s.toLowerCase() === stack)) return false;
      if (category && (e.category || "").toLowerCase() !== category) return false;
      if (scan && scanTier(e) !== scan) return false;
      if (q && !matchesQuery(e, q)) return false;
      return true;
    });
    const total = items.length;
    items = items.slice(0, limit).map(summary);
    return json(res, 200, { items, total });
  }

  // /skills/facets -> the catalog's real filter vocabulary, with counts.
  //
  // Must be answered BEFORE the slug parse below, which joins every remaining
  // segment into a slug: `facets` would otherwise be looked up as a skill and
  // 404. Reserving it costs nothing — catalog slugs are always `owner/name`, so
  // no single-segment path can ever name one.
  if (rest.length === 1 && rest[0] === "facets") {
    return json(res, 200, facets);
  }

  // /skills/<slug...>/resolve  or  /skills/<slug...>
  const isResolve = rest[rest.length - 1] === "resolve";
  const slug = (isResolve ? rest.slice(0, -1) : rest).join("/");
  const e = bySlug.get(slug);
  if (!e) return json(res, 404, { error: "not found" });

  if (isResolve) {
    const inst = installBySlug.get(slug);
    if (!inst) {
      // a collection / non-single-SKILL.md source
      return json(res, 200, {
        slug: e.slug,
        version: "",
        source_type: "github",
        source_url: e.source_url || "",
        content_hash: "",
        // No SKILL.md exists to scan, so there is genuinely no verdict.
        scan_tier: scanTier(e),
        tombstoned: true,
        tombstone_reason: `"${e.slug}" is a skill collection, not a single SKILL.md. Browse and install from the source repo: ${e.source_url}`,
      });
    }
    // The CLI calls resolve immediately before every install, so this is our
    // best server-side install-intent signal. Count it (best-effort) and skip
    // CDN caching on this response so counts aren't hidden behind the edge cache.
    await bumpInstall(slug);
    // `install` is the one place a tier changes what the user sees, so resolve
    // carries the evidence too: the CLI/ZeroShot prompt can name the exact line
    // that made a skill `medium` instead of saying "unverified".
    const report = scanReport(inst);
    return json(
      res,
      200,
      {
        slug: inst.slug,
        version: inst.content_hash,
        source_type: "github",
        source_url: inst.source_url || "",
        source_subpath: inst.skill_path || inst.source_subpath || "",
        pinned_commit: inst.pinned_commit || "",
        content_hash: inst.content_hash,
        scan_tier: scanTier(inst),
        signals_score: typeof inst.score === "number" ? inst.score : null,
        raw_content_url: inst.raw_content_url || "",
        tombstoned: false,
        ...(report ? { scan: report } : {}),
      },
      "no-store",
    );
  }

  // show
  const inst = installBySlug.get(slug);
  const report = scanReport(inst);
  return json(res, 200, {
    ...summary(e),
    versions: inst
      ? [
          {
            content_hash: inst.content_hash,
            pinned_commit: inst.pinned_commit || "",
            scan_tier: scanTier(inst),
            published_at: e.signals?.pushed_at || "",
          },
        ]
      : [],
    eval_cells: [],
    ...(report ? { scan: report } : {}),
  });
}
