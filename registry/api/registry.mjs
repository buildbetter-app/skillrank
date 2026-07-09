// Hosted SkillRank registry (read side) — serves the /v3/rest/skill-registry
// contract from the ingested public-skill catalog.
//
//   search / show  -> over ALL catalog entries (enriched.json, ~87)
//   resolve        -> installable entries carry pinned commit + content + hash
//                     (ingested.json, ~41); collections resolve to a tombstone
//                     pointing at the source repo.
//
// Content hashes were computed by the ingestion pipeline the SAME way as the Rust
// client (skillrank-core::hash), so `skillrank install` hash-verification passes.

import { readFileSync } from "node:fs";

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

function scanTier(e) {
  return e.status === "installable" ? "pending" : "unknown";
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

function json(res, status, body) {
  res.statusCode = status;
  res.setHeader("Content-Type", "application/json; charset=utf-8");
  res.setHeader("Cache-Control", "public, s-maxage=60, stale-while-revalidate=600");
  res.end(JSON.stringify(body));
}

export default function handler(req, res) {
  const url = new URL(req.url, "http://x");
  const path = url.searchParams.get("path") || "";
  const parts = path.split("/").filter(Boolean);

  // ---- eval suites: /eval-suites/:id  and  /eval-suites/:id/verifiers ----
  if (parts[0] === "eval-suites") {
    const id = parts[1] || "";
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
    const limit = Math.max(1, parseInt(url.searchParams.get("limit") || "20", 10) || 20);
    let items = sorted.filter((e) => {
      if (stack && !(e.tags || []).some((s) => s.toLowerCase() === stack)) return false;
      if (category && (e.category || "").toLowerCase() !== category) return false;
      if (q && !matchesQuery(e, q)) return false;
      return true;
    });
    const total = items.length;
    items = items.slice(0, limit).map(summary);
    return json(res, 200, { items, total });
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
        scan_tier: "unknown",
        tombstoned: true,
        tombstone_reason: `"${e.slug}" is a skill collection, not a single SKILL.md. Browse and install from the source repo: ${e.source_url}`,
      });
    }
    return json(res, 200, {
      slug: inst.slug,
      version: inst.content_hash,
      source_type: "github",
      source_url: inst.source_url || "",
      source_subpath: inst.skill_path || inst.source_subpath || "",
      pinned_commit: inst.pinned_commit || "",
      content_hash: inst.content_hash,
      scan_tier: "pending",
      signals_score: typeof inst.score === "number" ? inst.score : null,
      raw_content_url: inst.raw_content_url || "",
      tombstoned: false,
    });
  }

  // show
  const inst = installBySlug.get(slug);
  return json(res, 200, {
    ...summary(e),
    versions: inst
      ? [{ content_hash: inst.content_hash, pinned_commit: inst.pinned_commit || "", scan_tier: "pending", published_at: e.signals?.pushed_at || "" }]
      : [],
    eval_cells: [],
  });
}
