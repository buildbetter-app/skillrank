#!/usr/bin/env python3
"""Append safe, unflagged discovered skills to web/data/sources.seed.json.

Append-only: never rewrites or removes an existing entry. Skips anything
risk-flagged by discover-skills.py — those are always for a human to judge;
this script has no --include-flagged escape hatch on purpose.

  python3 scripts/add-discovered-skills.py --candidates .context/candidates.json --min-stars 30 [--dry-run]
"""

import argparse
import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SEED_PATH = REPO_ROOT / "web/data/sources.seed.json"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--candidates", default=".context/candidates.json")
    ap.add_argument("--min-stars", type=int, default=10)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    cand_path = Path(args.candidates)
    if not cand_path.is_absolute():
        cand_path = REPO_ROOT / cand_path
    if not cand_path.exists():
        print(f"no candidates file at {cand_path}, nothing to add")
        return

    data = json.loads(cand_path.read_text())
    candidates = data.get("candidates", data if isinstance(data, list) else [])

    seed = json.loads(SEED_PATH.read_text())
    known_slugs = {s["slug"].lower() for s in seed}
    known_pairs = {(s["source_repo"].lower(), s.get("subpath", "")) for s in seed}

    added, skipped_flagged, skipped_stars, skipped_dupe = [], [], [], []
    for c in candidates:
        if c.get("flagged"):
            skipped_flagged.append(c)
            continue
        if c.get("stars", 0) < args.min_stars:
            skipped_stars.append(c)
            continue
        key = (c["source_repo"].lower(), c.get("subpath", ""))
        if c["slug"].lower() in known_slugs or key in known_pairs:
            skipped_dupe.append(c)
            continue
        entry = {
            "slug": c["slug"],
            "name": c["name"],
            "source_repo": c["source_repo"],
            "source_url": c["source_url"],
            "subpath": c.get("subpath", ""),
            "category": c.get("category", "other"),
            "tags": c.get("tags", []),
            "description": c.get("description", ""),
            "stars": c.get("stars", 0),
            "tier": "community",
        }
        added.append(entry)
        known_slugs.add(entry["slug"].lower())
        known_pairs.add(key)

    print(
        f"{len(candidates)} candidates -> {len(added)} to add, "
        f"{len(skipped_flagged)} flagged, {len(skipped_stars)} under {args.min_stars}★, "
        f"{len(skipped_dupe)} already in catalog"
    )
    for e in added:
        print(f"  + {e['slug']}  (★{e['stars']}, {e['category']})")

    if args.dry_run:
        print("\n--dry-run: not writing sources.seed.json")
        return

    if not added:
        print("\nnothing new to add; sources.seed.json unchanged")
        return

    seed.extend(added)
    SEED_PATH.write_text(json.dumps(seed, indent=2) + "\n")
    print(f"\nwrote {SEED_PATH} ({len(seed)} total entries, +{len(added)})")


if __name__ == "__main__":
    main()
