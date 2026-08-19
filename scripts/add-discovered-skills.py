#!/usr/bin/env python3
"""Merge vetted discover-skills.py output into web/data/sources.seed.json.

Risk-flagged entries are held back by default — they need a human read before
they go in the catalog. Run web/data/build-catalog.mjs afterwards to rebuild.

  python3 scripts/discover-skills.py --min-stars 20
  python3 scripts/add-discovered-skills.py --dry-run
  python3 scripts/add-discovered-skills.py --min-stars 250
"""

import argparse
import importlib.util
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SEED = ROOT / "web" / "data" / "sources.seed.json"

# Reuse the discovery script's classifier so categories can be recomputed from
# a saved report without re-crawling GitHub.
_spec = importlib.util.spec_from_file_location(
    "discover_skills", Path(__file__).resolve().parent / "discover-skills.py")
_discover = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_discover)

STOPWORDS = {"the", "a", "an", "and", "or", "for", "with", "to", "of", "in", "on"}


def display_name(slug):
    return " ".join(w.capitalize() for w in re.split(r"[-_.]", slug) if w)


def tags_for(entry):
    """Three keyword-ish tags off the description, for catalog filtering."""
    words = re.findall(r"[a-z][a-z0-9+#.]{2,}", entry["description"].lower())
    seen, out = set(), []
    for w in words:
        w = w.strip(".")
        if w in STOPWORDS or w in seen or len(w) < 3:
            continue
        seen.add(w)
        out.append(w)
        if len(out) == 3:
            break
    return out or [entry["category"]]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--candidates", default=".context/candidates.json")
    ap.add_argument("--min-stars", type=int, default=20)
    ap.add_argument("--max-per-repo", type=int, default=25)
    ap.add_argument("--include-flagged", action="store_true",
                    help="also add entries carrying risk flags (review first)")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    report = json.loads(Path(args.candidates).read_text())
    seed = json.loads(SEED.read_text())
    known_slugs = {s["slug"].lower() for s in seed}
    known_pairs = {(s["source_repo"].lower(), s.get("subpath", "")) for s in seed}

    added, per_repo, skipped_flagged = [], {}, 0
    for e in sorted(report["entries"], key=lambda x: -x["stars"]):
        if e["stars"] < args.min_stars:
            continue
        if e["risk_flags"] and not args.include_flagged:
            skipped_flagged += 1
            continue
        slug = f"{e['owner']}/{e['skill_name']}"
        if slug.lower() in known_slugs:
            continue
        if (e["repo"].lower(), e["subpath"]) in known_pairs:
            continue
        if per_repo.get(e["repo"], 0) >= args.max_per_repo:
            continue

        known_slugs.add(slug.lower())
        known_pairs.add((e["repo"].lower(), e["subpath"]))
        per_repo[e["repo"]] = per_repo.get(e["repo"], 0) + 1
        category = _discover.categorize(f"{e['skill_name']} {e['description']}")
        added.append({
            "slug": slug,
            "name": display_name(e["skill_name"]),
            "source_repo": e["repo"],
            "source_url": e["source_url"],
            "subpath": e["subpath"],
            "category": category,
            "tags": tags_for(e),
            "description": e["description"],
            "stars": e["stars"],
            "tier": "community",
        })

    print(f"{len(added)} new entries from {len(per_repo)} repos"
          f" ({skipped_flagged} risk-flagged held back)")
    for a in added[:15]:
        print(f"  ★{a['stars']:>6} {a['slug']:48} [{a['category']}]")
    if len(added) > 15:
        print(f"  ... and {len(added) - 15} more")

    if args.dry_run:
        print("\ndry run — nothing written")
        return
    if not added:
        print("nothing to add")
        return

    SEED.write_text(json.dumps(added + seed, indent=2) + "\n")
    print(f"\nseed now {len(added) + len(seed)} entries -> run web/data/build-catalog.mjs")


if __name__ == "__main__":
    main()
