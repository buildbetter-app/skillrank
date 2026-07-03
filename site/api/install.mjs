// Serves the skillrank installer at the domain root.
//   curl / wget  ->  the install.sh script (text/plain), so `curl -fsSL skillrank.dev | sh` works
//   browsers     ->  a small landing page
//
// The script is fetched from the public repo so there is one source of truth;
// Vercel's edge caches it (and serves stale on an origin hiccup) so the installer
// stays fast and reliable.

const RAW_INSTALL_URL =
  "https://raw.githubusercontent.com/buildbetter-app/skillrank/main/install.sh";
const REPO_URL = "https://github.com/buildbetter-app/skillrank";

export default async function handler(req, res) {
  const ua = String(req.headers["user-agent"] || "").toLowerCase();
  const isBrowser = ua.includes("mozilla");

  if (isBrowser) {
    res.setHeader("Content-Type", "text/html; charset=utf-8");
    res.setHeader("Cache-Control", "public, s-maxage=3600");
    return res.status(200).send(landingHtml());
  }

  try {
    const r = await fetch(RAW_INSTALL_URL, { redirect: "follow" });
    if (!r.ok) throw new Error(`origin returned ${r.status}`);
    const script = await r.text();
    res.setHeader("Content-Type", "text/plain; charset=utf-8");
    res.setHeader(
      "Cache-Control",
      "public, s-maxage=300, stale-while-revalidate=86400"
    );
    return res.status(200).send(script);
  } catch (e) {
    res.setHeader("Content-Type", "text/plain; charset=utf-8");
    return res
      .status(502)
      .send(
        `# skillrank installer temporarily unavailable: ${e}\n# Install from source: ${REPO_URL}\n`
      );
  }
}

function landingHtml() {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>SkillRank — find, install & evaluate AI-agent skills</title>
<style>
  :root { color-scheme: dark; }
  * { box-sizing: border-box; }
  body { margin:0; min-height:100vh; display:grid; place-items:center;
    font: 16px/1.6 ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif;
    background:#0b0d10; color:#e6e8eb; padding:2rem; }
  main { max-width: 44rem; width:100%; }
  h1 { font-size: clamp(1.9rem, 4vw, 2.8rem); margin:0 0 .4rem; letter-spacing:-.02em; }
  p.tag { color:#9aa4af; margin:.2rem 0 1.6rem; font-size:1.05rem; }
  .cmd { display:flex; align-items:center; gap:.75rem; background:#14181d; border:1px solid #232a31;
    border-radius:12px; padding:1rem 1.15rem; font: 15px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace;
    overflow-x:auto; }
  .cmd code { color:#8fe3b0; white-space:nowrap; }
  .cmd button { margin-left:auto; background:#232a31; color:#e6e8eb; border:0; border-radius:8px;
    padding:.45rem .7rem; cursor:pointer; font-size:13px; }
  ul { color:#c3cad2; padding-left:1.1rem; margin:1.4rem 0; }
  li { margin:.35rem 0; }
  a { color:#7cc0ff; text-decoration:none; }
  a:hover { text-decoration:underline; }
  footer { margin-top:1.8rem; color:#6b7580; font-size:.9rem; }
</style>
</head>
<body>
<main>
  <h1>SkillRank</h1>
  <p class="tag">Find, install, and evaluate AI-agent skills — with real numbers. Open source, works on its own, integrates with BuildBetter ZeroShot.</p>
  <div class="cmd">
    <code id="c">curl -fsSL skillrank.dev | sh</code>
    <button onclick="navigator.clipboard.writeText(document.getElementById('c').textContent)">copy</button>
  </div>
  <ul>
    <li>Search a public registry and install skills into <code>.claude/skills</code> (hash-verified).</li>
    <li>Run paired evals on your own agent to see token/speed/success deltas.</li>
    <li><code>skillrank setup</code> registers it with Claude Code &amp; Codex — then just ask your agent.</li>
  </ul>
  <p><a href="${REPO_URL}">Source &amp; docs on GitHub →</a></p>
  <footer>MIT licensed · by BuildBetter</footer>
</main>
</body>
</html>`;
}
