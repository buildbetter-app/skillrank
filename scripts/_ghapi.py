"""Minimal GitHub REST API client shared by the discovery scripts.

Uses GITHUB_TOKEN / GH_TOKEN directly over HTTPS — this environment does not
have the `gh` CLI installed, unlike registry/ingest/*.mjs which shells out to
it. Keep this dependency-light (requests only) since these scripts run inside
an autonomous scheduled routine.
"""

import os
import sys
import time

import requests

API = "https://api.github.com"


def _token():
    return os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")


def session():
    tok = _token()
    if not tok:
        print("no usable GitHub token (GITHUB_TOKEN / GH_TOKEN not set)", file=sys.stderr)
        sys.exit(1)
    s = requests.Session()
    s.headers.update(
        {
            "Authorization": f"Bearer {tok}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        }
    )
    return s


def get(s, path, params=None, ok404=True):
    """GET a REST path (e.g. '/repos/owner/repo'). Retries once on rate limit."""
    for attempt in range(2):
        r = s.get(f"{API}{path}", params=params, timeout=30)
        if r.status_code == 200:
            return r.json()
        if r.status_code == 404 and ok404:
            return None
        if r.status_code in (403, 429) and "rate limit" in r.text.lower() and attempt == 0:
            reset = int(r.headers.get("X-RateLimit-Reset", time.time() + 30))
            wait = max(1, min(60, reset - int(time.time())))
            print(f"  rate limited, waiting {wait}s…", file=sys.stderr)
            time.sleep(wait)
            continue
        print(f"  GET {path} -> {r.status_code}: {r.text[:200]}", file=sys.stderr)
        return None
    return None


def search(s, endpoint, query, limit=100):
    """GET /search/<endpoint>?q=... paginated up to `limit` items."""
    items = []
    page = 1
    while len(items) < limit:
        per_page = min(100, limit - len(items))
        data = get(s, f"/search/{endpoint}", params={"q": query, "per_page": per_page, "page": page})
        if not data or not data.get("items"):
            break
        items.extend(data["items"])
        if len(data["items"]) < per_page:
            break
        page += 1
    return items[:limit]
