import Link from "next/link";
import type { Metadata } from "next";
import { type Block, inlineSpans, loadChangelog, type Span } from "../../lib/changelog";

export const metadata: Metadata = {
  title: "Changelog",
  description:
    "What changed in skillrank, release by release: static skill scanning and trust tiers, a checksum-verified self-updater, agent-initiated skill discovery, and effort metrics in evals.",
};

const REPO = "https://github.com/buildbetter-app/skillrank";

function Inline({ text }: { text: string }) {
  return (
    <>
      {inlineSpans(text).map((span: Span, i: number) =>
        span.code ? (
          <code key={i}>{span.text}</code>
        ) : span.bold ? (
          <b key={i}>{span.text}</b>
        ) : span.em ? (
          <em key={i}>{span.text}</em>
        ) : span.href ? (
          <a key={i} href={span.href}>
            {span.text}
          </a>
        ) : (
          <span key={i}>{span.text}</span>
        ),
      )}
    </>
  );
}

function Blocks({ blocks }: { blocks: Block[] }) {
  return (
    <>
      {blocks.map((block, i) => {
        if (block.kind === "heading") {
          return (
            <h3 className="cl-sub" key={i}>
              <Inline text={block.text} />
            </h3>
          );
        }
        if (block.kind === "list") {
          return (
            <ul className="cl-list" key={i}>
              {block.items.map((item, j) => (
                <li key={j}>
                  <Inline text={item} />
                </li>
              ))}
            </ul>
          );
        }
        return (
          <p key={i}>
            <Inline text={block.text} />
          </p>
        );
      })}
    </>
  );
}

export default function Changelog() {
  const changelog = loadChangelog();

  return (
    <>
      <header className="topbar">
        <Link className="brand" href="/">
          skillrank<span className="cur">_</span>
        </Link>
        <span className="kv hidesm">changelog</span>
        <span className="sp" />
        <span className="kv">MIT · open registry</span>
      </header>

      <main className="wrap detail-page">
        <section className="skill-hero">
          <p className="tag">changelog</p>
          <h1>What changed, and why it mattered.</h1>
          <p className="lede">
            Every release is a git tag on the <a href={REPO}>public repository</a>, published with
            signed binaries and a SHA-256 checksum beside each one. Update in place with{" "}
            <code>skillrank update</code> — since v0.1.5 the CLI also tells you when a newer release
            exists.
          </p>
        </section>

        {changelog ? (
          changelog.releases.map((release) => (
            // Each release is its own anchor, so a note or an issue can link to
            // the exact version it is talking about.
            <section className="install-panel cl-release" key={release.version} id={release.version}>
              <h2 className="cl-head">
                <a href={`${REPO}/releases/tag/${release.version}`}>{release.version}</a>
                <span className="cl-date">{release.date}</span>
              </h2>
              <Blocks blocks={release.blocks} />
            </section>
          ))
        ) : (
          <section className="install-panel cl-release">
            <h2 className="cl-head">releases</h2>
            <p>The full changelog lives in the repository, alongside the code it describes.</p>
            <p>
              <a href={`${REPO}/blob/main/CHANGELOG.md`}>Read CHANGELOG.md on GitHub ▸</a>
            </p>
            <p>
              <a href={`${REPO}/releases`}>Browse every release ▸</a>
            </p>
          </section>
        )}

        <p className="cl-foot">
          <Link href="/">← back to the registry</Link>
          <a href={`${REPO}/blob/main/CHANGELOG.md`}>CHANGELOG.md ▸</a>
          <a href={`${REPO}/releases`}>releases ▸</a>
        </p>
      </main>

      <nav className="keybar" aria-label="commands">
        <Link className="k" href="/">
          <kbd>r</kbd>registry
        </Link>
        <Link className="k" href="/how-it-works">
          <kbd>h</kbd>how it works
        </Link>
        <a className="k" href={REPO}>
          <kbd>g</kbd>github
        </a>
        <span className="sp" />
      </nav>
    </>
  );
}
