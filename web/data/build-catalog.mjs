// Builds catalog.json from sources.seed.json.
// v1 scoring is PROVISIONAL: we have real GitHub stars but no usage telemetry
// and no eval runs yet. So score = star signal only, capped at 75 — nothing can
// reach the top tier without a real eval (that's the SkillRank thesis). Eval
// numbers are backfilled later by the eval pipeline; until then eval == null.

import { readFileSync, writeFileSync } from "node:fs";

const src = JSON.parse(readFileSync(new URL("./sources.seed.json", import.meta.url), "utf8"));

// dedupe by source_repo + subpath (defensive)
const seen = new Set();
const rows = [];
for (const s of src) {
  const key = s.source_repo + "|" + (s.subpath || "");
  if (seen.has(key)) continue;
  seen.add(key);
  rows.push(s);
}

const maxStars = Math.max(...rows.map((r) => r.stars || 0));
const denom = Math.log10(maxStars + 1);
const PROVISIONAL_CAP = 75;

function provisionalScore(stars) {
  const c = Math.log10((stars || 0) + 1) / denom; // 0..1, log-scaled
  return Math.round(Math.min(PROVISIONAL_CAP, 74 * c));
}

const catalog = rows
  .map((s) => ({
    slug: s.slug,
    display_name: s.name,
    category: s.category,
    tags: s.tags || [],
    source_type: "github",
    source_url: s.source_url,
    source_repo: s.source_repo,
    source_subpath: s.subpath || "",
    description: s.description,
    signals: { stars: s.stars ?? null, installs: null },
    eval: { success_delta_pct: null, token_delta_pct: null, n_trials: null, tier: s.tier },
    score: provisionalScore(s.stars),
    provisional: true,
    added_at: "2026-07-07",
  }))
  .sort((a, b) => b.score - a.score || (b.signals.stars || 0) - (a.signals.stars || 0));

writeFileSync(new URL("./catalog.json", import.meta.url), JSON.stringify(catalog, null, 2) + "\n");

// ---- report ----
const byCat = {};
const byTier = {};
for (const s of catalog) {
  byCat[s.category] = (byCat[s.category] || 0) + 1;
  byTier[s.eval.tier] = (byTier[s.eval.tier] || 0) + 1;
}
console.log("catalog.json written:", catalog.length, "skills");
console.log("categories:", JSON.stringify(byCat));
console.log("tiers:", JSON.stringify(byTier));
console.log("top 8 (provisional):");
for (const s of catalog.slice(0, 8)) {
  console.log(`  ${String(s.score).padStart(2)}  ${s.slug}  (★${s.signals.stars}, ${s.eval.tier})`);
}
