import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  metadataBase: new URL("https://skillrank.dev"),
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
      </body>
    </html>
  );
}
