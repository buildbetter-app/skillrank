// Hosted SkillRank registry (read side) — serves the /v3/rest/skill-registry
// contract from the seed catalog, so the CLI/MCP work out of the box.
//
// Content hashes are computed the SAME way as the Rust client
// (skillrank-core::hash::compute_content_hash): normalize CRLF->LF, strip
// trailing newlines, sha256, "sha256:" prefix — so `install` verifies.

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

const catalog = JSON.parse(
  readFileSync(new URL("./seed_catalog.json", import.meta.url), "utf8")
);

function contentHash(content) {
  const normalized = content.split("\r\n").join("\n").replace(/\n+$/, "");
  return "sha256:" + createHash("sha256").update(normalized, "utf8").digest("hex");
}

// Precompute hashes + a slug index.
const entries = catalog.map((e) => ({ ...e, hash: contentHash(e.content) }));
const bySlug = new Map(entries.map((e) => [e.slug, e]));

function summary(e) {
  return {
    slug: e.slug,
    display_name: e.display_name,
    category: e.category || "",
    stacks: e.stacks || [],
    source_type: "github",
    source_url: e.source_url || "",
    latest_version: e.hash,
    scan_tier: "safe",
    rating_count: 0,
    summary: e.summary || "",
  };
}

const stripSep = (s) => s.replace(/[\s\-_]/g, "");

function matchesQuery(e, q) {
  const hay = [e.slug, e.display_name, e.summary, e.category, (e.stacks || []).join(" ")]
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
  // The path after /v3/rest/skill-registry is passed via ?path (see vercel.json).
  const url = new URL(req.url, "http://x");
  const path = url.searchParams.get("path") || "";
  const parts = path.split("/").filter(Boolean);

  if (parts[0] !== "skills") {
    return json(res, 404, { error: "not found" });
  }
  const rest = parts.slice(1);

  // /skills  -> search
  if (rest.length === 0) {
    const q = (url.searchParams.get("q") || "").toLowerCase();
    const stack = (url.searchParams.get("stack") || "").toLowerCase();
    const category = (url.searchParams.get("category") || "").toLowerCase();
    const limit = Math.max(1, parseInt(url.searchParams.get("limit") || "20", 10) || 20);
    let items = entries.filter((e) => {
      if (stack && !(e.stacks || []).some((s) => s.toLowerCase() === stack)) return false;
      if (category && (e.category || "").toLowerCase() !== category) return false;
      if (q && !matchesQuery(e, q)) return false;
      return true;
    });
    items.sort((a, b) => a.slug.localeCompare(b.slug));
    items = items.slice(0, limit).map(summary);
    return json(res, 200, { items, total: items.length });
  }

  // /skills/<slug...>/resolve  or  /skills/<slug...>
  const isResolve = rest[rest.length - 1] === "resolve";
  const slug = (isResolve ? rest.slice(0, -1) : rest).join("/");
  const e = bySlug.get(slug);
  if (!e) return json(res, 404, { error: "not found" });

  if (isResolve) {
    return json(res, 200, {
      slug: e.slug,
      version: e.hash,
      source_type: "github",
      source_url: e.source_url || "",
      content_hash: e.hash,
      scan_tier: "safe",
      inline_content: e.content,
      tombstoned: false,
    });
  }
  return json(res, 200, {
    ...summary(e),
    versions: [{ content_hash: e.hash, scan_tier: "safe" }],
    eval_cells: [],
  });
}
