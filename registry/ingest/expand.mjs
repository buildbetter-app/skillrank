// Expand discovered repos into individual, installable per-skill entries, with
// GLOBAL content-dedup. Mega-dumps mostly re-copy other repos' skills, so hashing
// each SKILL.md and dropping duplicates collapses them to their unique skills —
// and we never add the dump repo itself, only the underlying skills.
//
// Downloads each repo's tarball once (fast), extracts every SKILL.md, hashes it
// the SAME way as install, dedupes against existing catalog + within this run,
// and APPENDS new entries to web/data/sources.seed.json. Run on a branch, review
// as a PR.
//
//   node registry/ingest/expand.mjs [--budget 1500] [--max-per-repo 200] [--skip-over 150] [--max-repo-mb 40]

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync, mkdtempSync, rmSync, readdirSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "../..");
const arg = (n, d) => {
  const i = process.argv.indexOf(`--${n}`);
  return i >= 0 ? process.argv[i + 1] : d;
};
const BUDGET = parseInt(arg("budget", "1500"), 10);
const MAX_PER_REPO = parseInt(arg("max-per-repo", "200"), 10);
const SKIP_OVER = parseInt(arg("skip-over", "150"), 10); // defer true mega-dumps to a later pass
const MAX_REPO_MB = parseInt(arg("max-repo-mb", "40"), 10);

const TOKEN = execFileSync("gh", ["auth", "token"], { encoding: "utf8" }).trim();
function gh(a) {
  try {
    return JSON.parse(execFileSync("gh", ["api", ...a], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }));
  } catch {
    return null;
  }
}
function contentHash(c) {
  return "sha256:" + createHash("sha256").update(c.split("\r\n").join("\n").replace(/\n+$/, ""), "utf8").digest("hex");
}
function frontmatter(c) {
  const s = c.replace(/^﻿/, "");
  if (!s.startsWith("---")) return {};
  const end = s.indexOf("\n---", 3);
  const out = {};
  for (const line of s.slice(3, end < 0 ? undefined : end).split("\n")) {
    const m = line.match(/^(name|description):\s*(.+)$/);
    if (m) out[m[1]] = m[2].trim().replace(/^["']|["']$/g, "");
  }
  return out;
}
const CAT = [
  ["testing", /test|playwright|e2e|vitest|jest|\bqa\b/i], ["security", /security|sast|fuzz|pentest|vuln|semgrep|codeql|threat/i],
  ["devops", /docker|kubernet|k8s|terraform|\baws\b|\bci\b|deploy|infra|pulumi|helm/i], ["data", /\bsql\b|database|\bcsv\b|pandas|\bd3\b|visualiz|dataset/i],
  ["ai", /\bmcp\b|\bllm\b|prompt|\brag\b|model|agent-sdk|anthropic|openai/i], ["document", /\bpdf\b|docx|epub|markdown|writing|\bdocs\b|slides|pptx|xlsx/i],
  ["styling", /design|figma|theme|brand|motion|css|tailwind/i], ["frontend", /react|\bvue\b|svelte|frontend|next\.?js|expo|swiftui|component/i],
  ["backend", /\bapi\b|server|fastapi|django|rails|backend|graphql/i], ["accessibility", /a11y|wcag|accessib/i],
  ["meta", /skill|workflow|planning|review|thinking|framework|orchestrat|subagent/i],
];
const guessCat = (t) => (CAT.find(([, re]) => re.test(t)) || ["other"])[0];
const slugify = (s) => s.toLowerCase().replace(/[^a-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 60);

function walk(dir, out = []) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (e.name === "SKILL.md") out.push(p);
  }
  return out;
}

const discovered = JSON.parse(readFileSync(path.join(ROOT, "registry/data/discovered.json"), "utf8"));
const sources = JSON.parse(readFileSync(path.join(ROOT, "web/data/sources.seed.json"), "utf8"));

// dedup baselines
const seenSlugs = new Set(sources.map((s) => s.slug.toLowerCase()));
// repos already represented in the catalog — skip them in later batches (their
// skills are already in), unless --include-done.
const INCLUDE_DONE = process.argv.includes("--include-done");
const doneRepos = new Set(sources.map((s) => s.source_repo));
const seenHashes = new Set();
try {
  for (const e of JSON.parse(readFileSync(path.join(ROOT, "registry/data/ingested.json"), "utf8"))) {
    if (e.content_hash) seenHashes.add(e.content_hash);
  }
} catch {}

const added = [];
let dupContent = 0,
  dupSlug = 0,
  budgetLeft = BUDGET;

for (const repo of discovered) {
  if (budgetLeft <= 0) break;
  if (!INCLUDE_DONE && doneRepos.has(repo.source_repo)) continue; // already cataloged
  if ((repo.skill_count || 0) > SKIP_OVER) continue; // defer mega-dumps
  const meta = gh([`repos/${repo.source_repo}`]);
  if (!meta || meta.message) continue;
  if ((meta.size || 0) > MAX_REPO_MB * 1024) continue; // skip giant app repos
  const branch = meta.default_branch || "main";

  const tmp = mkdtempSync(path.join(tmpdir(), "sr-exp-"));
  try {
    const tgz = path.join(tmp, "r.tgz");
    execFileSync("curl", ["-fsSL", "-H", `Authorization: token ${TOKEN}`, `https://api.github.com/repos/${repo.source_repo}/tarball/${branch}`, "-o", tgz], { stdio: "ignore" });
    execFileSync("tar", ["-xzf", tgz, "-C", tmp], { stdio: "ignore" });
    const roots = readdirSync(tmp).filter((n) => statSync(path.join(tmp, n)).isDirectory());
    const base = roots.length ? path.join(tmp, roots[0]) : tmp;
    let perRepo = 0;
    for (const file of walk(base)) {
      if (perRepo >= MAX_PER_REPO || budgetLeft <= 0) break;
      const content = readFileSync(file, "utf8");
      if (content.trim().length < 40) continue;
      const hash = contentHash(content);
      if (seenHashes.has(hash)) {
        dupContent++;
        continue;
      }
      const rel = path.relative(base, file); // e.g. skills/foo/SKILL.md
      const dir = path.dirname(rel);
      const leaf = dir === "." ? meta.name : path.basename(dir);
      const fm = frontmatter(content);
      let slug = `${repo.source_repo.split("/")[0]}/${slugify(fm.name && fm.name.length <= 40 ? fm.name : leaf)}`;
      if (seenSlugs.has(slug.toLowerCase())) {
        // disambiguate with repo name
        slug = `${repo.source_repo.split("/")[0]}/${slugify(meta.name + "-" + leaf)}`;
        if (seenSlugs.has(slug.toLowerCase())) {
          dupSlug++;
          continue;
        }
      }
      const desc = (fm.description || "").split(/(?<=[.!?])\s/)[0].slice(0, 180).trim() || `Skill from ${repo.source_repo}.`;
      const entry = {
        slug,
        name: (fm.name && fm.name.length <= 40 ? fm.name : leaf).trim(),
        source_repo: repo.source_repo,
        source_url: dir === "." ? repo.source_url : `https://github.com/${repo.source_repo}/tree/${branch}/${dir}`,
        subpath: dir === "." ? "" : dir,
        category: guessCat(`${slug} ${desc} ${rel}`),
        tags: (repo.tags || []).slice(0, 2),
        description: desc,
        stars: repo.stars || 0,
        tier: "community",
      };
      added.push(entry);
      seenSlugs.add(slug.toLowerCase());
      seenHashes.add(hash);
      perRepo++;
      budgetLeft--;
    }
    if (perRepo) console.log(`  +${String(perRepo).padStart(3)}  ${repo.source_repo} (★${repo.stars})`);
  } catch {
    /* skip repo on any tar/curl failure */
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

writeFileSync(path.join(ROOT, "web/data/sources.seed.json"), JSON.stringify([...sources, ...added], null, 2) + "\n");
writeFileSync(path.join(ROOT, "registry/data/expanded.json"), JSON.stringify(added, null, 2) + "\n");
console.log(`\nadded ${added.length} unique skills (deduped: ${dupContent} identical content, ${dupSlug} slug clashes)`);
console.log(`catalog: ${sources.length} -> ${sources.length + added.length}`);
