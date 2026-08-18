#!/usr/bin/env python3
"""Discover candidate agent skills on GitHub and write a gated candidates file.

Two ways to source candidate repos:
  --since-days N     search GitHub (topic + SKILL.md code search) for repos
                      pushed in the last N days
  --repos-file PATH  skip search; check exactly these "owner/repo" lines

Either way, every candidate repo is verified to actually ship a SKILL.md,
deduped against web/data/sources.seed.json, gated by --min-stars, capped at
--max-skills-per-repo skills per repo, and risk-scanned. Nothing here writes
to the catalog — this only produces a review file for add-discovered-skills.py.

  python3 scripts/discover-skills.py --since-days 3 --min-stars 30 \
      --max-skills-per-repo 25 --out .context/candidates.json
  python3 scripts/discover-skills.py --repos-file .context/skills_sh_repos.txt \
      --min-stars 30 --max-skills-per-repo 25 --out .context/candidates.json
"""

import argparse
import base64
import json
import re
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

from _ghapi import get, search, session

REPO_ROOT = Path(__file__).resolve().parent.parent

TOPIC_QUERIES = [
    "topic:claude-skills",
    "topic:agent-skills",
    "topic:claude-code-skills",
    "topic:claude-skill",
    "topic:agent-skill",
    "topic:claude-code-skill",
]

# Index/collection repos re-host other people's skills — mine them for links
# elsewhere (registry/ingest/expand.mjs), never queue the collection itself.
AGGREGATOR_RE = re.compile(r"awesome-|-awesome|\bawesome\b|marketplace|registry|collection", re.I)

CAT_RULES = [
    ("testing", re.compile(r"test|playwright|e2e|vitest|jest|\bqa\b|cypress", re.I)),
    ("security", re.compile(r"security|sast|fuzz|pentest|vuln|ffuf|semgrep|codeql|threat", re.I)),
    ("devops", re.compile(r"docker|kubernet|k8s|terraform|\baws\b|\bci\b|deploy|infra|pulumi|helm|n8n", re.I)),
    ("data", re.compile(r"\bsql\b|database|\bcsv\b|pandas|\bd3\b|visualiz|dataset|analytics", re.I)),
    ("ai", re.compile(r"\bmcp\b|\bllm\b|prompt|\brag\b|model|agent-sdk|anthropic|openai", re.I)),
    ("document", re.compile(r"\bpdf\b|docx|epub|markdown|writing|\bdocs\b|slides|pptx|xlsx", re.I)),
    ("styling", re.compile(r"design|figma|theme|brand|motion|\bart\b|css|tailwind", re.I)),
    ("frontend", re.compile(r"react|\bvue\b|svelte|frontend|next\.?js|\bui\b|expo|swiftui|component", re.I)),
    ("backend", re.compile(r"\bapi\b|server|fastapi|django|rails|backend|graphql|endpoint", re.I)),
    ("accessibility", re.compile(r"a11y|wcag|accessib", re.I)),
    ("meta", re.compile(r"skill|workflow|planning|review|thinking|framework|orchestrat|subagent", re.I)),
]

# Risk flags: fetch-and-execute and secret-access patterns a human should judge.
RISK_RULES = [
    ("curl_pipe_shell", re.compile(r"curl\s+[^\n|]*\|\s*(sudo\s+)?(sh|bash|zsh)\b")),
    ("wget_pipe_shell", re.compile(r"wget\s+[^\n|]*\|\s*(sudo\s+)?(sh|bash|zsh)\b")),
    ("eval_curl", re.compile(r"eval\s*[\"'`]?\s*\$\(\s*(curl|wget)")),
    ("ssh_key_read", re.compile(r"\.ssh/(id_rsa|id_ed25519|id_ecdsa|authorized_keys)")),
    ("anthropic_key_access", re.compile(r"ANTHROPIC_API_KEY")),
]


def guess_category(text):
    for cat, rx in CAT_RULES:
        if rx.search(text):
            return cat
    return "other"


def risk_scan(content):
    flags = []
    for name, rx in RISK_RULES:
        if rx.search(content):
            flags.append(name)
    return flags


def load_known(known_names):
    seed_path = REPO_ROOT / "web/data/sources.seed.json"
    seed = json.loads(seed_path.read_text())
    known_repos = {s["source_repo"].lower() for s in seed}
    for s in seed:
        known_names.add(s["name"].strip().lower())
    return known_repos


def collect_via_search(s, min_stars, since_days, limit):
    candidates = {}

    def add(full):
        if not full or "/" not in full:
            return
        if AGGREGATOR_RE.search(full):
            return
        candidates.setdefault(full.lower(), full)

    date_filter = ""
    if since_days:
        since = (datetime.now(timezone.utc) - timedelta(days=since_days)).strftime("%Y-%m-%d")
        date_filter = f" pushed:>={since}"

    for q in TOPIC_QUERIES:
        query = f"{q}{date_filter} stars:>={min_stars}"
        for r in search(s, "repositories", query, limit=60):
            add(r["full_name"])

    for h in search(s, "code", "filename:SKILL.md", limit=limit):
        repo = h.get("repository") or {}
        add(repo.get("full_name"))

    return list(candidates.values())


def load_repos_file(path):
    repos = []
    for line in Path(path).read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            repos.append(line)
    return repos


def find_skill_md_paths(s, full, branch):
    tree = get(s, f"/repos/{full}/git/trees/{branch}", params={"recursive": "1"})
    if not tree or tree.get("truncated") and not tree.get("tree"):
        pass
    if not tree:
        return []
    return [t["path"] for t in tree.get("tree", []) if t["path"].endswith("SKILL.md")]


def make_slug(owner, repo, subpath, taken):
    if not subpath:
        base = f"{owner}/{repo}"
    else:
        skill_dir = subpath.rstrip("/").split("/")[-1]
        base = f"{owner}/{skill_dir}"
    slug = base
    n = 2
    while slug.lower() in taken:
        slug = f"{base}-{n}"
        n += 1
    taken.add(slug.lower())
    return slug


def fetch_raw(s, full, branch, path):
    data = get(s, f"/repos/{full}/contents/{path}", params={"ref": branch})
    if not data or "content" not in data:
        return ""
    try:
        return base64.b64decode(data["content"]).decode("utf8", errors="replace")
    except Exception:
        return ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--since-days", type=int, default=None)
    ap.add_argument("--min-stars", type=int, default=10)
    ap.add_argument("--max-skills-per-repo", type=int, default=25)
    ap.add_argument("--max-repos", type=int, default=150)
    ap.add_argument("--repos-file")
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    s = session()
    known_names = set()
    known_repos = load_known(known_names)

    if args.repos_file:
        repo_list = [r for r in load_repos_file(args.repos_file) if r.lower() not in known_repos]
        print(f"checking {len(repo_list)} repos from {args.repos_file} (min {args.min_stars}★)…")
    else:
        repo_list = [r for r in collect_via_search(s, args.min_stars, args.since_days, args.max_repos) if r.lower() not in known_repos]
        print(f"found {len(repo_list)} new candidate repos via search (min {args.min_stars}★)…")

    since_cutoff = None
    if args.since_days:
        since_cutoff = datetime.now(timezone.utc) - timedelta(days=args.since_days)

    candidates = []
    checked = 0
    taken_slugs = set()
    for full in repo_list[: args.max_repos]:
        checked += 1
        meta = get(s, f"/repos/{full}")
        if not meta or meta.get("fork") or meta.get("archived"):
            continue
        stars = meta.get("stargazers_count", 0)
        if stars < args.min_stars:
            continue
        if since_cutoff and meta.get("pushed_at"):
            pushed = datetime.strptime(meta["pushed_at"], "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
            if pushed < since_cutoff:
                continue

        branch = meta.get("default_branch", "main")
        paths = find_skill_md_paths(s, full, branch)
        if not paths:
            continue
        paths = paths[: args.max_skills_per_repo]

        owner, repo = full.split("/", 1)
        hay = f"{full} {meta.get('description') or ''} {' '.join(meta.get('topics') or [])}"
        for p in paths:
            subpath = p[: -len("/SKILL.md")] if p != "SKILL.md" else ""
            content = fetch_raw(s, full, branch, p)
            flags = risk_scan(content)
            name_guess = subpath.rstrip("/").split("/")[-1] if subpath else repo
            if name_guess.strip().lower() in known_names:
                flags.append("name_collision")
            slug = make_slug(owner, repo, subpath, taken_slugs)
            candidates.append(
                {
                    "slug": slug,
                    "name": name_guess.replace("-", " ").replace("_", " ").title(),
                    "source_repo": full,
                    "source_url": f"{meta['html_url']}/tree/{branch}/{subpath}" if subpath else meta["html_url"],
                    "subpath": subpath,
                    "category": guess_category(hay + " " + name_guess),
                    "tags": (meta.get("topics") or [])[:3],
                    "description": (meta.get("description") or "")[:180],
                    "stars": stars,
                    "forks": meta.get("forks_count", 0),
                    "pushed_at": meta.get("pushed_at", ""),
                    "flagged": bool(flags),
                    "flag_reasons": flags,
                }
            )

    candidates.sort(key=lambda c: c["stars"], reverse=True)
    out = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "since_days": args.since_days,
        "min_stars": args.min_stars,
        "repos_checked": checked,
        "candidates": candidates,
    }
    out_path = REPO_ROOT / args.out if not Path(args.out).is_absolute() else Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(out, indent=2) + "\n")

    unflagged = [c for c in candidates if not c["flagged"]]
    flagged = [c for c in candidates if c["flagged"]]
    print(f"\n{len(candidates)} candidate skills ({len(unflagged)} clean, {len(flagged)} flagged)")
    for c in unflagged[:25]:
        print(f"  ★{c['stars']:>5}  {c['slug']:<45} [{c['category']}]")
    if len(unflagged) > 25:
        print(f"  … and {len(unflagged) - 25} more clean")
    for c in flagged:
        print(f"  FLAGGED {c['slug']}: {', '.join(c['flag_reasons'])}", file=sys.stderr)
    print(f"\nwrote {out_path}")


if __name__ == "__main__":
    main()
