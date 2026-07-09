// Discovery scraper: find NEW agent-skill repos on GitHub and queue them for
// review. Sources: (1) GitHub repo search by topic/keyword, (2) code search for
// SKILL.md files, (3) crawling known "awesome-*" lists. Applies a quality gate
// (min stars, not fork/archived), dedupes against the current catalog, and
// writes candidates to registry/data/discovered.json as a PENDING queue.
//
// Nothing here auto-publishes. A human promotes entries into sources.seed.json
// via a reviewed PR (GitHub-native moderation).
//
//   node registry/ingest/discover.mjs [--min-stars 40] [--max 300]

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, "../..");

const args = process.argv.slice(2);
const argVal = (name, def) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : def;
};
const MIN_STARS = parseInt(argVal("min-stars", "10"), 10); // SKILL.md presence is the real gate
const MAX_CANDIDATES = parseInt(argVal("max", "600"), 10);

// ---- gh helpers ----
function gh(args, { json = true } = {}) {
  try {
    const out = execFileSync("gh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], maxBuffer: 32 * 1024 * 1024 });
    return json ? JSON.parse(out) : out;
  } catch {
    return null;
  }
}

// ---- known set: what's already in the catalog + what to never treat as a skill ----
const sources = JSON.parse(readFileSync(path.join(REPO_ROOT, "web/data/sources.seed.json"), "utf8"));
const knownRepos = new Set(sources.map((s) => s.source_repo.toLowerCase()));

// Aggregators / lists / marketplaces are indexes, not installable skills — mine
// them for links but never queue them as skills themselves.
const AWESOME_LISTS = [
  "VoltAgent/awesome-agent-skills",
  "bergside/awesome-design-skills",
  "ComposioHQ/awesome-claude-skills",
  "BehiSecc/awesome-claude-skills",
  "heilcheng/awesome-agent-skills",
  "travisvn/awesome-claude-skills",
  "hesreallyhim/awesome-claude-code",
  "abubakarsiddik31/claude-skills-collection",
  "majiayu000/claude-skill-registry",
];
const EXCLUDE = new Set(AWESOME_LISTS.map((r) => r.toLowerCase()));
const isAggregator = (full) =>
  EXCLUDE.has(full.toLowerCase()) || /awesome-|-awesome|\bawesome\b/i.test(full) || /marketplace|registry|collection/i.test(full);

// ---- candidate collection ----
const candidates = new Map(); // fullName(lower) -> {full, via:Set}
function addCandidate(full, via) {
  if (!full || !full.includes("/")) return;
  const key = full.toLowerCase();
  if (knownRepos.has(key) || isAggregator(full)) return;
  if (!candidates.has(key)) candidates.set(key, { full, via: new Set() });
  candidates.get(key).via.add(via);
}

// 1) repo search — topic-based only (keyword search pulls in big non-skill apps
// that merely mention "skill/agent"; the SKILL.md verification below is the real
// precision filter, but keeping queries tight reduces wasted verification calls).
const REPO_QUERIES = [
  "topic:claude-skills",
  "topic:agent-skills",
  "topic:claude-code-skills",
  "topic:claude-skill",
  "topic:agent-skill",
  "topic:claude-code-skill",
];
for (const q of REPO_QUERIES) {
  const res = gh(["search", "repos", q, "--limit", "60", "--sort", "stars", "--json", "fullName"]) || [];
  for (const r of res) addCandidate(r.fullName, "repo-search");
}

// 2) code search for SKILL.md files
const codeHits = gh(["search", "code", "filename:SKILL.md", "--limit", "100", "--json", "repository"]) || [];
for (const h of codeHits) addCandidate(h.repository?.fullName, "code-search");

// 3) crawl awesome-list READMEs for github repo links
for (const list of AWESOME_LISTS) {
  const readme = gh(["api", `repos/${list}/readme`, "--jq", ".content"], { json: false });
  if (!readme) continue;
  const text = Buffer.from(readme, "base64").toString("utf8");
  const re = /github\.com\/([A-Za-z0-9._-]+\/[A-Za-z0-9._-]+)/g;
  let m;
  while ((m = re.exec(text))) {
    const full = m[1].replace(/\.git$/, "").replace(/[).,#]+$/, "");
    if (full.split("/").length === 2) addCandidate(full, `list:${list.split("/")[1]}`);
  }
}

console.log(`collected ${candidates.size} raw candidates; fetching metadata + gating (min ${MIN_STARS}★)…`);

// ---- category guess ----
const CAT_RULES = [
  ["testing", /test|playwright|e2e|vitest|jest|\bqa\b|cypress/i],
  ["security", /security|sast|fuzz|pentest|vuln|ffuf|semgrep|codeql|threat/i],
  ["devops", /docker|kubernet|k8s|terraform|\baws\b|\bci\b|deploy|infra|pulumi|helm|n8n/i],
  ["data", /\bsql\b|database|\bcsv\b|pandas|\bd3\b|visualiz|dataset|analytics/i],
  ["ai", /\bmcp\b|\bllm\b|prompt|\brag\b|model|agent-sdk|anthropic|openai/i],
  ["document", /\bpdf\b|docx|epub|markdown|writing|\bdocs\b|slides|pptx|xlsx/i],
  ["styling", /design|figma|theme|brand|motion|\bart\b|css|tailwind/i],
  ["frontend", /react|\bvue\b|svelte|frontend|next\.?js|\bui\b|expo|swiftui|component/i],
  ["backend", /\bapi\b|server|fastapi|django|rails|backend|graphql|endpoint/i],
  ["accessibility", /a11y|wcag|accessib/i],
  ["meta", /skill|workflow|planning|review|thinking|framework|orchestrat|subagent/i],
];
function guessCategory(text) {
  for (const [cat, re] of CAT_RULES) if (re.test(text)) return cat;
  return "other";
}

// ---- gate + enrich ----
const discovered = [];
let checked = 0;
for (const { full, via } of candidates.values()) {
  if (checked >= MAX_CANDIDATES) break;
  checked++;
  const meta = gh(["api", `repos/${full}`]);
  if (!meta || meta.message) continue;
  if (meta.fork || meta.archived) continue;
  const stars = meta.stargazers_count || 0;
  if (stars < MIN_STARS) continue;

  // PRECISION FILTER: the repo must actually contain a SKILL.md. This is what
  // separates real skills from big apps that merely mention "skill".
  const branch = meta.default_branch || "main";
  const paths =
    gh(["api", `repos/${full}/git/trees/${branch}?recursive=1`, "--jq", '[.tree[].path | select(endswith("SKILL.md"))]']) || [];
  if (!Array.isArray(paths) || paths.length === 0) continue;

  const hay = `${full} ${meta.description || ""} ${(meta.topics || []).join(" ")}`;
  discovered.push({
    slug: full,
    name: meta.name,
    source_repo: full,
    source_url: meta.html_url,
    subpath: paths.length === 1 ? paths[0].replace(/\/SKILL\.md$/, "") : "",
    category: guessCategory(hay),
    tags: (meta.topics || []).slice(0, 3),
    description: (meta.description || "").slice(0, 180),
    stars,
    forks: meta.forks_count || 0,
    pushed_at: meta.pushed_at || "",
    skill_count: paths.length,
    sample_skill: paths[0],
    tier: "community",
    status: "pending",
    found_via: [...via],
  });
}

discovered.sort((a, b) => b.stars - a.stars);
writeFileSync(path.join(REPO_ROOT, "registry/data/discovered.json"), JSON.stringify(discovered, null, 2) + "\n");

console.log(`\n${discovered.length} NEW skill repos (verified SKILL.md, not already in catalog):`);
for (const d of discovered.slice(0, 45)) {
  console.log(
    `  ★${String(d.stars).padStart(6)}  ${String(d.skill_count).padStart(3)} skills  ${d.slug.padEnd(40)} [${d.category}]`
  );
}
if (discovered.length > 45) console.log(`  … and ${discovered.length - 45} more`);
console.log(`\nwrote registry/data/discovered.json (pending review queue)`);
