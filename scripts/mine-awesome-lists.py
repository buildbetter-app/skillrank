#!/usr/bin/env python3
"""Harvest repo links out of "awesome agent skills" READMEs.

These lists are skipped as skill *sources* (they re-host other people's
SKILL.md files under the wrong author), but the repos they link to are exactly
the long tail worth carrying. This pulls every github.com/owner/repo link out
of their READMEs and writes a repo list for discover-skills.py --repos-file.

  python3 scripts/mine-awesome-lists.py --out .context/awesome_repos.txt
"""

import argparse
import json
import re
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SEED = ROOT / "web" / "data" / "sources.seed.json"

# Owners that host the lists themselves, plus platform paths that look like
# repos but aren't.
NON_REPO = {"sponsors", "topics", "collections", "features", "about", "pricing",
            "marketplace", "orgs", "users", "settings", "notifications", "explore",
            "login", "join", "apps", "site", "readme", "security", "trending"}

FIND_LIST_QUERIES = ["awesome claude skills", "awesome agent skills",
                     "awesome claude code", "curated agent skills"]


def token():
    out = subprocess.run(["gh", "auth", "token"], capture_output=True, text=True)
    return out.stdout.strip()


TOKEN = None


def fetch(url, raw=False):
    cmd = ["curl", "-sSL", "--max-time", "30", "-H", f"Authorization: Bearer {TOKEN}"]
    if not raw:
        cmd += ["-H", "Accept: application/vnd.github+json"]
    cmd.append(url)
    out = subprocess.run(cmd, capture_output=True, text=True).stdout
    if raw:
        return out
    try:
        return json.loads(out)
    except json.JSONDecodeError:
        return None


def find_lists():
    """Repos that look like curated skill lists."""
    found = {}
    for q in FIND_LIST_QUERIES:
        data = fetch("https://api.github.com/search/repositories?q="
                     f"{q.replace(' ', '+')}&sort=stars&order=desc&per_page=50")
        for r in (data or {}).get("items", []):
            hay = f"{r['full_name']} {r.get('description') or ''}".lower()
            if any(h in hay for h in ("awesome", "curated", "list")):
                found[r["full_name"]] = r.get("stargazers_count", 0)
    return found


def readme_links(repo):
    """Every github.com/owner/repo referenced by a repo's README."""
    meta = fetch(f"https://api.github.com/repos/{repo}/readme")
    if not meta or "download_url" not in meta:
        return set()
    text = fetch(meta["download_url"], raw=True) or ""
    out = set()
    for owner, name in re.findall(
            r"github\.com/([A-Za-z0-9](?:[A-Za-z0-9-]{0,38}))/"
            r"([A-Za-z0-9_.-]{1,100})", text):
        name = name.rstrip(".git").rstrip(".")
        if owner.lower() in NON_REPO or not name or name.startswith("."):
            continue
        out.add(f"{owner}/{name}")
    return out


def main():
    global TOKEN
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=".context/awesome_repos.txt")
    ap.add_argument("--extra-lists", nargs="*", default=[])
    args = ap.parse_args()
    TOKEN = token()

    lists = find_lists()
    for extra in args.extra_lists:
        lists.setdefault(extra, 0)
    print(f"{len(lists)} candidate lists", flush=True)

    all_links = set()
    with ThreadPoolExecutor(max_workers=8) as ex:
        for repo, links in zip(lists, ex.map(readme_links, lists)):
            if links:
                print(f"  {repo}: {len(links)} links", flush=True)
            all_links |= links

    known = {s["source_repo"].lower() for s in json.loads(SEED.read_text())}
    listed = {l.lower() for l in lists}
    new = sorted(l for l in all_links
                 if l.lower() not in known and l.lower() not in listed)

    Path(args.out).write_text("\n".join(new) + "\n")
    print(f"\n{len(all_links)} linked repos, {len(new)} not already in the catalog")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
