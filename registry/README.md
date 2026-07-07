# api.skillrank.dev

The hosted **read-side** registry. It serves the `/v3/rest/skill-registry`
contract (search / show / resolve) from the [seed catalog](api/seed_catalog.json)
as a single Vercel function, so `skillrank search` / `install` / `recommend` and
the MCP tools work out of the box with no local `serve`.

Content hashes are computed exactly like the Rust client
(`skillrank-core::hash::compute_content_hash`), so `install` hash-verification
passes. The catalog mirrors `crates/skillrank/src/seed_catalog.json`.

This is the MVP hosted registry (catalog-backed, read-only). Publishing, ratings,
reviews, and eval ingest are the full backend (see the private BuildBetter
implementation); they can replace this function without changing the CLI.

## Deploy

```sh
vercel deploy --prod       # then attach the domain:
vercel domains add api.skillrank.dev
```

## Test

```sh
BASE=https://<deployment>.vercel.app
curl -s "$BASE/v3/rest/skill-registry/skills?q=react" | jq .
SKILLRANK_API_URL=$BASE skillrank search react
```
