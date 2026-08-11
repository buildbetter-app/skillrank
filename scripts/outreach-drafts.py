#!/usr/bin/env python3
"""Draft reply text for skills we just added to the catalog.

Emits one ready-to-paste line per skill, so whoever is running outreach can
review the batch and post it themselves. Nothing here posts anywhere.

  python3 scripts/outreach-drafts.py --candidates .context/candidates.json --min-stars 30
"""

import argparse
import json
from pathlib import Path

TEMPLATE = ("this looks sick, added it to skillrank.dev "
            "made it faster to install: `skillrank install {slug}`")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--candidates", default=".context/candidates.json")
    ap.add_argument("--min-stars", type=int, default=30)
    ap.add_argument("--limit", type=int, default=25)
    ap.add_argument("--format", choices=["text", "markdown", "json"], default="markdown")
    args = ap.parse_args()

    report = json.loads(Path(args.candidates).read_text())
    seen, drafts = set(), []
    # One draft per repo: several skills from the same project would otherwise
    # turn into several near-identical replies to the same author.
    for e in sorted(report["entries"], key=lambda x: -x["stars"]):
        if e["stars"] < args.min_stars or e["risk_flags"] or e["repo"] in seen:
            continue
        seen.add(e["repo"])
        slug = f"{e['owner']}/{e['skill_name']}"
        drafts.append({
            "repo": e["repo"],
            "repo_url": f"https://github.com/{e['repo']}",
            "slug": slug,
            "stars": e["stars"],
            "reply": TEMPLATE.format(slug=slug),
        })
        if len(drafts) >= args.limit:
            break

    if args.format == "json":
        print(json.dumps(drafts, indent=2))
    elif args.format == "text":
        for d in drafts:
            print(f"{d['repo_url']}\n  {d['reply']}\n")
    else:
        print(f"### {len(drafts)} outreach drafts (review before posting)\n")
        print("| repo | stars | reply |")
        print("|---|---|---|")
        for d in drafts:
            print(f"| [{d['repo']}]({d['repo_url']}) | {d['stars']} | {d['reply']} |")


if __name__ == "__main__":
    main()
