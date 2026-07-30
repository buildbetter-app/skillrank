import { readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * The changelog is authored once, at the repo root, and rendered here — a second
 * hand-maintained copy on the site would drift from the one people read on
 * GitHub, and the drift would always favour the site being stale.
 *
 * Reading it means reaching outside `web/`, which every other data source here
 * deliberately avoids (`../data/catalog.json` lives inside the deployed root).
 * Vercel builds this app with `web/` as its root directory, and whether files
 * above it are present depends on a project setting we do not control from the
 * repo. So a missing file is treated as an ordinary outcome, not an error: the
 * page falls back to pointing at GitHub rather than failing the build. A
 * changelog page is not worth a broken deploy.
 */
const CHANGELOG_PATH = join(process.cwd(), "..", "CHANGELOG.md");

export type Block =
  | { kind: "heading"; text: string }
  | { kind: "paragraph"; text: string }
  | { kind: "list"; items: string[] };

export type Release = { version: string; date: string; blocks: Block[] };
export type Changelog = { intro: Block[]; releases: Release[] } | null;

/** `## v0.1.5 — 2026-07-29` → version + date, tolerating either dash. */
function parseReleaseHeading(line: string): { version: string; date: string } | null {
  const m = /^##\s+(v[0-9][^\s—-]*)\s*[—-]\s*(.+?)\s*$/.exec(line);
  return m ? { version: m[1], date: m[2] } : null;
}

export function loadChangelog(): Changelog {
  let raw: string;
  try {
    raw = readFileSync(CHANGELOG_PATH, "utf8");
  } catch {
    return null;
  }

  const intro: Block[] = [];
  const releases: Release[] = [];
  // Blocks accumulate into the preamble until the first `## v…` heading, then
  // into whichever release is currently open.
  let target = intro;
  let paragraph: string[] = [];
  let list: string[] = [];

  const flushParagraph = () => {
    const text = paragraph.join(" ").trim();
    paragraph = [];
    if (text) target.push({ kind: "paragraph", text });
  };
  const flushList = () => {
    if (!list.length) return;
    target.push({ kind: "list", items: list });
    list = [];
  };
  const flushAll = () => {
    flushParagraph();
    flushList();
  };

  for (const line of raw.split("\n")) {
    const trimmed = line.trim();

    if (trimmed.startsWith("# ")) continue; // the file's own title; the page has one

    const release = parseReleaseHeading(trimmed);
    if (release) {
      flushAll();
      releases.push({ ...release, blocks: [] });
      target = releases[releases.length - 1].blocks;
      continue;
    }

    if (trimmed.startsWith("### ")) {
      flushAll();
      target.push({ kind: "heading", text: trimmed.slice(4) });
      continue;
    }

    if (trimmed.startsWith("- ")) {
      flushParagraph();
      list.push(trimmed.slice(2));
      continue;
    }

    if (!trimmed) {
      flushAll();
      continue;
    }

    // A list item wrapped onto a continuation line belongs to that item.
    if (list.length) list[list.length - 1] += ` ${trimmed}`;
    else paragraph.push(trimmed);
  }
  flushAll();

  return { intro, releases };
}

export type Span = {
  text: string;
  code?: boolean;
  bold?: boolean;
  em?: boolean;
  href?: string;
};

/**
 * Inline markdown, restricted to what the changelog uses: `code`, **bold**,
 * *emphasis*, and [links](url). Returning spans rather than HTML keeps this away
 * from `dangerouslySetInnerHTML` — the source is a repo file today, and a
 * renderer that cannot inject markup stays safe if that ever stops being true.
 *
 * Anything outside that subset (fences, tables, blockquotes) falls through as
 * literal text. That is deliberate: a construct we do not handle should look
 * wrong and get fixed, not vanish from the page.
 *
 * Order matters — `**bold**` is listed before `*em*` so the two-asterisk form
 * wins at the same position.
 *
 * Code spans follow CommonMark's delimiter-run rule rather than "no backticks
 * inside": a run of N backticks closes on the next run of exactly N. The
 * changelog relies on it to quote a fence (`` ` ```bash ` ``), which a naive
 * `[^`]+` splits into two wrong chips. The lookarounds are what make the run
 * exact — without them the closing `\1` happily matches the first backtick of a
 * longer run and the span ends in the middle of the quoted fence.
 */
const INLINE =
  /(?<!`)(`+)([\s\S]*?)(?<!`)\1(?!`)|\*\*([^*]+)\*\*|\*([^*\s][^*]*)\*|\[([^\]]+)\]\(([^)\s]+)\)/g;

/** CommonMark strips one space from each end when the span has both. */
function stripCodePadding(content: string): string {
  if (content.length > 2 && content.startsWith(" ") && content.endsWith(" ") && content.trim()) {
    return content.slice(1, -1);
  }
  return content;
}

export function inlineSpans(text: string): Span[] {
  const spans: Span[] = [];
  let last = 0;
  let m: RegExpExecArray | null;
  INLINE.lastIndex = 0;
  while ((m = INLINE.exec(text))) {
    if (m.index > last) spans.push({ text: text.slice(last, m.index) });
    if (m[2] !== undefined) spans.push({ text: stripCodePadding(m[2]), code: true });
    else if (m[3] !== undefined) spans.push({ text: m[3], bold: true });
    else if (m[4] !== undefined) spans.push({ text: m[4], em: true });
    else {
      // Only http(s) becomes a real href. A relative or exotic scheme renders as
      // plain text instead — a changelog is not worth an injection surface, even
      // one whose source is a file in this repo.
      const href = /^https?:\/\//i.test(m[6]) ? m[6] : undefined;
      spans.push(href ? { text: m[5], href } : { text: m[0] });
    }
    last = m.index + m[0].length;
  }
  if (last < text.length) spans.push({ text: text.slice(last) });
  return spans;
}
