import type { Metadata } from "next";
import { Analytics } from "@vercel/analytics/next";
import { SITE_URL } from "../lib/site";
import "./globals.css";

export const metadata: Metadata = {
  // www, not the apex — the apex 308s to www, so emitting apex URLs in the
  // canonical and OG tags would point every crawler at a redirect.
  metadataBase: new URL(SITE_URL),
  title: {
    default: "skillrank -- the registry & benchmark for agent skills",
    template: "%s | skillrank"
  },
  description:
    "A retro-terminal registry for agent skills, ranked with community stars and provisional SkillRank scores while evals are pending."
};

export default function RootLayout({
  children
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body>
        <div className="crt" aria-hidden="true" />
        <div className="crt-flicker" aria-hidden="true" />
        {children}
        <Analytics />
      </body>
    </html>
  );
}
