#!/usr/bin/env python3
"""Discover agent-skill repos on GitHub that aren't in the catalog yet.

Searches GitHub for skill-shaped repos, drops anything already in
web/data/sources.seed.json, then verifies each candidate actually ships a
SKILL.md with valid frontmatter. Emits a JSON report of vetted candidates,
each risk-scanned for fetch-and-execute patterns.

Used two ways:
  - one-off backfills (`--min-stars 20`)
  - the weekly monitoring routine (`--since-days 14`), which files what it
    finds for human review.

Auth: needs a GitHub token via GITHUB_TOKEN, GH_TOKEN, or `gh auth token`.

  python3 scripts/discover-skills.py --min-stars 20 --out .context/candidates.json
"""

import argparse
import json
import os
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SEED = ROOT / "web" / "data" / "sources.seed.json"

# Search terms that surface skill repos. GitHub caps each query's result set,
# so several overlapping phrasings beat one broad query.
QUERIES = [
    "agent skills",
    "claude skills",
    "claude code skill",
    "SKILL.md",
    "agent skill",
    "claude-code skills",
    "codex skills",
    "ai agent skills",
]

# Fetch-and-execute patterns. A hit doesn't prove malice, but a skill that
# pipes the network into a shell needs eyes on it before we list it.
DANGER_PATTERNS = [
    (r"curl[^\n|]*\|\s*(sudo\s+)?(ba)?sh", "curl|sh"),
    (r"wget[^\n|]*\|\s*(sudo\s+)?(ba)?sh", "wget|sh"),
    (r"eval\s+\"?\$\(\s*curl", "eval $(curl)"),
    (r"base64\s+-d[^\n]*\|\s*(ba)?sh", "base64|sh"),
    # Root itself, not any absolute path: deleting a specific system directory
    # is ordinary cleanup.
    (r"\bsudo\s+rm\s+-rf\s+/(\s|\*|$)", "sudo rm -rf /"),
    # Only the private half, and only when something is reading it. Registering
    # a .pub key or running ssh-add is ordinary setup, not exfiltration.
    # Reading the private half into something that could ship it elsewhere.
    # `scp -i` / `ssh -i` pass the key to the client for auth, which is normal.
    (r"(cat|curl|echo|base64|upload|xxd)[^\n]{0,40}"
     r"~/\.ssh/id_[a-z0-9_]+(?!\.pub)", "reads ssh private key"),
    (r"\bANTHROPIC_API_KEY\b", "touches ANTHROPIC_API_KEY"),
]

# Keyword -> catalog category. First match wins, and matching is on whole words
# so "database" stops counting as a hit for "data". Order runs most specific to
# most generic: "agent" would otherwise swallow nearly everything into "ai".
CATEGORY_RULES = [
    ("testing", ["test", "tests", "testing", "playwright", "e2e", "pytest", "jest", "qa", "tdd"]),
    ("security", ["security", "secure", "vulnerability", "vulnerabilities", "pentest",
                  "owasp", "threat", "auth", "authentication", "encryption", "cve"]),
    ("devops", ["deploy", "deployment", "docker", "kubernetes", "k8s", "terraform",
                "ci", "cd", "infrastructure", "aws", "devops", "pipeline"]),
    ("styling", ["design", "css", "tailwind", "animation", "motion", "figma",
                 "theme", "styling", "ux", "typography"]),
    ("frontend", ["react", "vue", "svelte", "nextjs", "frontend", "component",
                  "ui", "browser", "dom", "remotion", "video"]),
    ("backend", ["backend", "server", "django", "rails", "microservice", "grpc",
                 "database", "sql", "postgres", "redis", "api", "endpoint"]),
    ("data", ["data", "analytics", "etl", "pandas", "dataset", "scrape",
              "scraping", "dataframe", "visualization"]),
    ("document", ["writing", "write", "docs", "documentation", "pdf", "markdown",
                  "slides", "report", "blog", "article", "content", "copy"]),
    ("ai", ["llm", "prompt", "rag", "mcp", "openai", "gpt", "claude", "agent",
            "agents", "model", "embedding", "inference"]),
    ("meta", ["skill", "skills", "workflow", "plan", "planning", "review",
              "commit", "git", "refactor", "debug", "productivity"]),
]
CATEGORY_MATCHERS = [
    (cat, re.compile(r"\b(?:%s)\b" % "|".join(re.escape(w) for w in words)))
    for cat, words in CATEGORY_RULES
]


def token():
    for var in ("GITHUB_TOKEN", "GH_TOKEN"):
        if os.environ.get(var):
            return os.environ[var]
    out = subprocess.run(["gh", "auth", "token"], capture_output=True, text=True)
    if out.returncode == 0 and out.stdout.strip():
        return out.stdout.strip()
    sys.exit("no GitHub token: set GITHUB_TOKEN or run `gh auth login`")


TOKEN = None


def fetch(url, accept="application/vnd.github+json"):
    cmd = ["curl", "-sSL", "--max-time", "30", "-H", f"Accept: {accept}",
           "-H", f"Authorization: Bearer {TOKEN}", url]
    out = subprocess.run(cmd, capture_output=True, text=True).stdout
    if accept == "raw":
        return out
    try:
        return json.loads(out)
    except json.JSONDecodeError:
        return None


def raw_file(repo, branch, path):
    cmd = ["curl", "-sSL", "--max-time", "30",
           "-H", f"Authorization: Bearer {TOKEN}",
           f"https://raw.githubusercontent.com/{repo}/{branch}/{path}"]
    return subprocess.run(cmd, capture_output=True, text=True).stdout


def search_repos(query, sort, pages=3):
    """GitHub search, paginated. Returns raw repo dicts."""
    found = []
    for page in range(1, pages + 1):
        q = query.replace(" ", "+")
        url = (f"https://api.github.com/search/repositories?q={q}"
               f"&sort={sort}&order=desc&per_page=100&page={page}")
        data = fetch(url)
        if not data or "items" not in data:
            break
        found.extend(data["items"])
        if len(data["items"]) < 100:
            break
    return found


def _yaml_value(block, key):
    """Read one scalar out of a frontmatter block.

    Handles plain `key: value` plus YAML block scalars (`>`, `>-`, `|`, `|-`),
    which real SKILL.md files use often enough that ignoring them yields
    descriptions literally reading ">-".
    """
    m = re.search(rf"^{key}:[ \t]*(.*)$", block, re.M)
    if not m:
        return None
    head = m.group(1).strip()
    if head and head[0] not in "|>":
        return head.strip("\"'") or None

    # Block scalar: take the following more-indented lines and fold them.
    rest = block[m.end():].lstrip("\n").splitlines()
    indent, lines = None, []
    for line in rest:
        if not line.strip():
            lines.append("")
            continue
        pad = len(line) - len(line.lstrip())
        if indent is None:
            indent = pad
        if pad < indent:
            break
        lines.append(line[indent:])
    folded = ("\n" if head.startswith("|") else " ").join(lines)
    return " ".join(folded.split()) or None


def parse_frontmatter(text):
    m = re.match(r"^---\s*\n(.*?)\n---", text, re.S)
    if not m:
        return None, None
    block = m.group(1)
    return _yaml_value(block, "name"), _yaml_value(block, "description")


def categorize(skill_text, repo_text=""):
    """Classify from the skill's own name/description first.

    The repo blurb is only a fallback: letting it vote first files every skill
    in a security-focused monorepo under "security", however unrelated.
    """
    for source in (skill_text, repo_text):
        low = source.lower()
        for category, matcher in CATEGORY_MATCHERS:
            if matcher.search(low):
                return category
    return "other"


def scan_risk(text):
    return [label for pattern, label in DANGER_PATTERNS if re.search(pattern, text, re.I)]


# Curated "awesome" lists mostly re-host other people's SKILL.md files. Listing
# them would credit the wrong author and duplicate skills we already carry, so
# they're held back unless explicitly requested.
COLLECTION_HINTS = ("awesome", "curated", "skills-collection", "skill-collection")


def is_collection(repo_row):
    haystack = f"{repo_row['full_name']} {repo_row.get('description') or ''}".lower()
    return any(h in haystack for h in COLLECTION_HINTS)


def valid_skill_name(name):
    """Catalog slugs are lowercase kebab-case; anything else installs badly."""
    return bool(re.fullmatch(r"[a-z0-9][a-z0-9._-]{0,63}", name or ""))


# A SKILL.md under one of these is test data, not a publishable skill. Security
# scanners in particular ship deliberately malicious fixtures to test against.
EXCLUDED_PATH_PARTS = re.compile(
    r"(^|/)(tests?|__tests__|fixtures?|__fixtures__|spec|specs|"
    r"node_modules|vendor|\.git|dist|build)(/|$)", re.I)


def publishable_path(path):
    return not EXCLUDED_PATH_PARTS.search(path)


def inspect(repo_row, max_skills):
    """Return skill entries for one repo, or [] if it ships no usable SKILL.md."""
    repo = repo_row["full_name"]
    branch = repo_row.get("default_branch") or "main"
    tree = fetch(f"https://api.github.com/repos/{repo}/git/trees/{branch}?recursive=1")
    if not tree or "tree" not in tree:
        return []
    paths = [t["path"] for t in tree["tree"]
             if (t["path"] == "SKILL.md" or t["path"].endswith("/SKILL.md"))
             and publishable_path(t["path"])]
    if not paths:
        return []

    truncated = len(paths) > max_skills
    entries = []
    for path in sorted(paths)[:max_skills]:
        text = raw_file(repo, branch, path)
        name, desc = parse_frontmatter(text)
        if not name or not desc:
            continue
        subpath = "" if path == "SKILL.md" else path[: -len("/SKILL.md")]
        if len(desc) > 280:
            desc = desc[:277].rsplit(" ", 1)[0] + "..."
        if not valid_skill_name(name):
            continue
        entries.append({
            "repo": repo,
            "owner": repo.split("/")[0],
            "skill_name": name,
            "description": desc,
            "subpath": subpath,
            "source_url": (f"https://github.com/{repo}/tree/{branch}/{subpath}"
                           if subpath else f"https://github.com/{repo}"),
            "stars": repo_row.get("stargazers_count", 0),
            "license": (repo_row.get("license") or {}).get("spdx_id"),
            "pushed_at": repo_row.get("pushed_at"),
            "category": categorize(f"{name} {desc}", repo_row.get("description") or ""),
            "risk_flags": scan_risk(text),
            "skills_in_repo": len(paths),
            "repo_truncated": truncated,
        })
    return entries


def main():
    global TOKEN
    ap = argparse.ArgumentParser()
    ap.add_argument("--min-stars", type=int, default=20)
    ap.add_argument("--since-days", type=int, default=0,
                    help="only repos pushed within N days (0 = no limit)")
    ap.add_argument("--max-skills-per-repo", type=int, default=40)
    ap.add_argument("--max-repos", type=int, default=0, help="0 = no cap")
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--allow-collections", action="store_true",
                    help="include awesome-list repos that re-host others' skills")
    ap.add_argument("--repos-file",
                    help="newline-separated owner/repo list to vet instead of "
                         "searching GitHub (e.g. repos mined from skills.sh)")
    ap.add_argument("--out", default=".context/candidates.json")
    args = ap.parse_args()

    TOKEN = token()

    seed = json.loads(SEED.read_text())
    known_repos = {s["source_repo"].lower() for s in seed}
    known_slugs = {s["slug"].lower() for s in seed}
    print(f"catalog: {len(seed)} entries, {len(known_repos)} distinct repos", flush=True)

    # --- discover ---
    pool = {}
    if args.repos_file:
        names = [ln.strip() for ln in Path(args.repos_file).read_text().splitlines()
                 if ln.strip() and "/" in ln]
        names = [n for n in names if n.lower() not in known_repos]
        print(f"vetting {len(names)} repos from {args.repos_file}", flush=True)
        with ThreadPoolExecutor(max_workers=args.workers) as ex:
            for meta in ex.map(lambda n: fetch(f"https://api.github.com/repos/{n}"), names):
                if meta and meta.get("full_name"):
                    pool[meta["full_name"].lower()] = meta
        print(f"  {len(pool)} repos resolved", flush=True)
    else:
        for query in QUERIES:
            for sort in ("stars", "updated"):
                hits = search_repos(query, sort)
                for r in hits:
                    pool[r["full_name"].lower()] = r
                print(f"  search {query!r} [{sort}]: +{len(hits)} (pool {len(pool)})", flush=True)

    cutoff = None
    if args.since_days:
        cutoff = datetime.now(timezone.utc) - timedelta(days=args.since_days)

    candidates, collections = [], []
    for r in pool.values():
        if r["full_name"].lower() in known_repos:
            continue
        if r.get("stargazers_count", 0) < args.min_stars:
            continue
        if r.get("archived") or r.get("fork"):
            continue
        if is_collection(r) and not args.allow_collections:
            collections.append(r["full_name"])
            continue
        if cutoff and r.get("pushed_at"):
            if datetime.fromisoformat(r["pushed_at"].replace("Z", "+00:00")) < cutoff:
                continue
        candidates.append(r)

    candidates.sort(key=lambda r: -r.get("stargazers_count", 0))
    if args.max_repos:
        candidates = candidates[: args.max_repos]
    print(f"\n{len(candidates)} new repos >= {args.min_stars} stars; verifying SKILL.md...", flush=True)

    # --- verify ---
    entries, done = [], 0
    with ThreadPoolExecutor(max_workers=args.workers) as ex:
        for got in ex.map(lambda r: inspect(r, args.max_skills_per_repo), candidates):
            entries.extend(got)
            done += 1
            if done % 50 == 0:
                print(f"  {done}/{len(candidates)} repos -> {len(entries)} skills", flush=True)

    # A repo can be new while an owner/name slug collides with something listed.
    entries = [e for e in entries
               if f"{e['owner']}/{e['skill_name']}".lower() not in known_slugs]

    # Same skill name under a different owner is usually a re-host, not a new
    # skill. Keep ours, and surface the rest as needing a human call.
    known_names = {s["slug"].split("/", 1)[1].lower() for s in seed}
    for e in entries:
        if e["skill_name"].lower() in known_names:
            e["risk_flags"] = e["risk_flags"] + ["name already in catalog (possible re-host)"]

    with_skills = len({e["repo"] for e in entries})
    flagged = [e for e in entries if e["risk_flags"]]

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "min_stars": args.min_stars,
        "repos_searched": len(pool),
        "repos_new": len(candidates),
        "collections_skipped": collections,
        "repos_with_skills": with_skills,
        "skills_found": len(entries),
        "skills_flagged": len(flagged),
        "entries": entries,
    }, indent=2) + "\n")

    print(f"\n{len(entries)} skills across {with_skills} repos "
          f"({len(candidates) - with_skills} repos had no usable SKILL.md)")
    print(f"{len(flagged)} carry risk flags")
    if collections:
        print(f"{len(collections)} collection repos skipped (use --allow-collections)")
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
