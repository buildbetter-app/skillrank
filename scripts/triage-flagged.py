#!/usr/bin/env python3
"""Pull the offending lines out of risk-flagged skills so a human can judge them.

discover-skills.py flags a SKILL.md when it matches a fetch-and-execute pattern,
but a match alone says nothing about intent: `curl -fsSL https://bun.sh/install
| bash` in a setup snippet is not the same as piping an unknown host into a
shell. This re-fetches each flagged skill, extracts the matching line with
surrounding context, and sorts by how much scrutiny it actually needs.

  python3 scripts/triage-flagged.py --flagged .context/flagged_security.json
"""

import argparse
import importlib.util
import json
import re
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

_spec = importlib.util.spec_from_file_location(
    "discover_skills", Path(__file__).resolve().parent / "discover-skills.py")
_discover = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_discover)

# Hosts whose install one-liners are ubiquitous in READMEs. A pipe-to-shell
# from one of these is the documented way to install the tool, not a red flag.
TRUSTED_HOSTS = {
    "bun.sh", "deno.land", "sh.rustup.rs", "get.docker.com", "install.python-poetry.org",
    "raw.githubusercontent.com", "get.pnpm.io", "astral.sh", "ollama.com", "ollama.ai",
    "sdk.cloud.google.com", "cli.github.com", "starship.rs", "get.volta.sh",
    "nixos.org", "brew.sh", "fnm.vercel.app", "bootstrap.pypa.io", "uv.astral.sh",
    "claude.ai", "storage.googleapis.com", "aws.amazon.com", "install.determinate.systems",
}

SEVERITY = {
    "sudo rm -rf /": 3,
    "eval $(curl)": 3,
    "base64|sh": 3,
    "reads ssh private key": 3,
    "curl|sh": 2,
    "wget|sh": 2,
    "touches ANTHROPIC_API_KEY": 1,
}


def context_lines(text, patterns, pad=1):
    """Return (line_no, matched_line, surrounding) for each pattern hit."""
    lines = text.splitlines()
    hits = []
    for pattern, label in patterns:
        for i, line in enumerate(lines):
            if re.search(pattern, line, re.I):
                lo, hi = max(0, i - pad), min(len(lines), i + pad + 1)
                hits.append({
                    "label": label,
                    "line_no": i + 1,
                    "line": line.strip()[:200],
                    "context": "\n".join(lines[lo:hi])[:400],
                })
                break
    return hits


def hosts_in(line):
    return set(re.findall(r"https?://([a-zA-Z0-9._-]+)", line))


def judge(hits):
    """Rough verdict so the reviewer can skip the obvious cases."""
    worst = max((SEVERITY.get(h["label"], 1) for h in hits), default=1)
    pipe_hits = [h for h in hits if h["label"] in ("curl|sh", "wget|sh")]
    if pipe_hits:
        all_hosts = set()
        for h in pipe_hits:
            all_hosts |= hosts_in(h["line"])
        if all_hosts and all_hosts <= TRUSTED_HOSTS:
            worst = min(worst, 1)  # documented installer for a known tool
        elif not all_hosts:
            worst = max(worst, 2)  # piping something we can't attribute
    return {3: "review-closely", 2: "needs-eyes", 1: "likely-benign"}[worst]


def inspect(entry):
    branch = "main"
    text = _discover.raw_file(entry["repo"], branch,
                              f"{entry['subpath']}/SKILL.md" if entry["subpath"] else "SKILL.md")
    if not text or text.lstrip().startswith("404"):
        text = _discover.raw_file(entry["repo"], "master",
                                  f"{entry['subpath']}/SKILL.md" if entry["subpath"] else "SKILL.md")
    wanted = [(p, l) for p, l in _discover.DANGER_PATTERNS if l in entry["risk_flags"]]
    hits = context_lines(text or "", wanted)
    return {
        "slug": f"{entry['owner']}/{entry['skill_name']}",
        "repo": entry["repo"],
        "stars": entry["stars"],
        "url": entry["source_url"],
        "flags": entry["risk_flags"],
        "verdict": judge(hits) if hits else "pattern-not-found",
        "hits": hits,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--flagged", default=".context/flagged_security.json")
    ap.add_argument("--out", default=".context/triage.json")
    ap.add_argument("--workers", type=int, default=10)
    args = ap.parse_args()

    _discover.TOKEN = _discover.token()
    entries = json.loads(Path(args.flagged).read_text())
    print(f"triaging {len(entries)} flagged skills...", flush=True)

    results = []
    with ThreadPoolExecutor(max_workers=args.workers) as ex:
        for i, r in enumerate(ex.map(inspect, entries), 1):
            results.append(r)
            if i % 50 == 0:
                print(f"  {i}/{len(entries)}", flush=True)

    order = {"review-closely": 0, "needs-eyes": 1, "likely-benign": 2, "pattern-not-found": 3}
    results.sort(key=lambda r: (order[r["verdict"]], -r["stars"]))
    Path(args.out).write_text(json.dumps(results, indent=2) + "\n")

    from collections import Counter
    print("\nverdicts:", Counter(r["verdict"] for r in results).most_common())
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
