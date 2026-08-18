#!/usr/bin/env python3
"""Generate draft outreach text for newly discovered skill authors.

Drafts ONLY — nothing here sends anything. Output is a markdown table meant
to be pasted into a PR for a human to review and send by hand.

  python3 scripts/outreach-drafts.py --candidates .context/candidates.json --min-stars 30 --limit 25
"""

import argparse
import json
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

DRAFT_TEMPLATE = (
    "Hi! We came across {name} ({source_repo}) and think it'd be a great fit for "
    "SkillRank (skillrank.dev), an open registry for agent skills. Would you be "
    "OK with us listing it with a link back to your repo? Happy to remove it "
    "any time if you'd rather not."
)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--candidates", default=".context/candidates.json")
    ap.add_argument("--min-stars", type=int, default=10)
    ap.add_argument("--limit", type=int, default=25)
    ap.add_argument("--out")
    args = ap.parse_args()

    cand_path = Path(args.candidates)
    if not cand_path.is_absolute():
        cand_path = REPO_ROOT / cand_path
    if not cand_path.exists():
        print(f"no candidates file at {cand_path}, nothing to draft")
        return

    data = json.loads(cand_path.read_text())
    candidates = data.get("candidates", data if isinstance(data, list) else [])
    eligible = [c for c in candidates if not c.get("flagged") and c.get("stars", 0) >= args.min_stars]
    eligible.sort(key=lambda c: c.get("stars", 0), reverse=True)
    eligible = eligible[: args.limit]

    lines = ["| Skill | Repo | Stars | Draft |", "|---|---|---|---|"]
    for c in eligible:
        draft = DRAFT_TEMPLATE.format(name=c["name"], source_repo=c["source_repo"])
        lines.append(f"| {c['slug']} | {c['source_repo']} | {c.get('stars', 0)} | {draft} |")

    table = "\n".join(lines) if eligible else "_no eligible candidates for outreach this run_"
    print(f"outreach drafts: {len(eligible)} (of {len(candidates)} candidates, min {args.min_stars}★)\n")
    print(table)

    if args.out:
        out_path = Path(args.out)
        if not out_path.is_absolute():
            out_path = REPO_ROOT / out_path
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(table + "\n")
        print(f"\nwrote {out_path}")


if __name__ == "__main__":
    main()
