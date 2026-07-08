import { NextResponse } from "next/server";
import type { NextRequest } from "next/server";

// `curl skillrank.dev | sh` must get the install script, while browsers get the
// site. We branch on User-Agent: anything that looks like a browser passes
// through to the page; everything else (curl, wget, fetch…) receives install.sh,
// proxied from the repo so it always tracks main.
const RAW_INSTALL_URL =
  "https://raw.githubusercontent.com/buildbetter-app/skillrank/main/install.sh";

// Browser engines send one of these tokens; CLI fetchers do not.
const BROWSER = /mozilla|chrome|safari|firefox|edge|opera|gecko|webkit/i;

export const config = {
  // Only the root and the explicit /install.sh path; everything else is the app.
  matcher: ["/", "/install.sh"],
};

export default async function middleware(req: NextRequest) {
  const ua = req.headers.get("user-agent") || "";
  const path = req.nextUrl.pathname;
  const wantsScript = path === "/install.sh" || (path === "/" && !BROWSER.test(ua));

  if (!wantsScript) return NextResponse.next();

  const upstream = await fetch(RAW_INSTALL_URL, { redirect: "follow" });
  if (!upstream.ok) {
    return new NextResponse(
      `# skillrank installer temporarily unavailable (upstream ${upstream.status}).\n` +
        `# Try again shortly, or: curl -fsSL ${RAW_INSTALL_URL} | sh\n`,
      { status: 502, headers: { "content-type": "text/plain; charset=utf-8" } }
    );
  }
  const script = await upstream.text();
  return new NextResponse(script, {
    status: 200,
    headers: {
      "content-type": "text/plain; charset=utf-8",
      "cache-control": "public, s-maxage=300, stale-while-revalidate=86400",
    },
  });
}
