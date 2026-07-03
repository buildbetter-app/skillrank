# skillrank.dev

The site behind `skillrank.dev`. It serves the installer at the domain **root** so
`curl -fsSL skillrank.dev | sh` works (no `/install.sh` path needed), the same way
`buildbetter.sh` does.

A single Vercel function (`api/install.mjs`) at `/` inspects the User-Agent:

- `curl` / `wget` → the [`install.sh`](../install.sh) script as `text/plain`
- browsers → a small landing page

The script is fetched from the public repo (one source of truth) and cached at
Vercel's edge (`stale-while-revalidate`), so the installer stays fast and survives
a brief origin hiccup.

## Deploy

From this directory:

```sh
vercel deploy            # preview URL — test with: curl -fsSL <preview-url>
vercel deploy --prod     # production
```

Then attach the domain (already pointing at Vercel):

```sh
vercel domains add skillrank.dev
# or in the dashboard: Project → Settings → Domains → add skillrank.dev
```

`/install.sh` continues to work for back-compat.
