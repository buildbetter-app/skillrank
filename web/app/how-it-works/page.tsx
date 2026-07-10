import Link from "next/link";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "How it works — evals & scoring",
  description:
    "How SkillRank measures whether an agent skill actually helps: paired control-vs-treatment trials on reproducible suites, hidden verification, a pinned reference model, and trust tiers.",
};

export default function HowItWorks() {
  return (
    <>
      <header className="topbar">
        <Link className="brand" href="/">
          skillrank<span className="cur">_</span>
        </Link>
        <span className="kv hidesm">how it works</span>
        <span className="sp" />
        <span className="kv">MIT · open registry</span>
      </header>

      <main className="wrap detail-page">
        <section className="skill-hero">
          <p className="tag">how it works</p>
          <h1>What the score means, and how the evals actually run.</h1>
          <p className="lede">
            Most skill lists rank by GitHub stars. Stars measure popularity, not whether a skill makes your agent
            better. SkillRank ranks by <b>measured impact</b> — and where a skill has been benchmarked, the number is a
            real, reproducible eval, not a vibe.
          </p>
        </section>

        <section className="detail-grid" aria-label="scoring">
          <div className="pane">
            <p className="glabel">the SkillRank score (0–100)</p>
            <p>
              Blends three signals: <b>community</b> (GitHub stars, log-scaled), <b>usage</b> (real installs), and our{" "}
              <b>eval lift</b> (measured token/success impact).
            </p>
            <p>
              Until a skill is evaluated its score is <b>provisional</b> and <b>capped at 75</b> — popularity alone can
              never reach the top tier. That cap is the whole point: a skill has to be <em>measured</em> to break out.
            </p>
          </div>
          <div className="pane">
            <p className="glabel">evaluated skills</p>
            <p>
              Once a skill is benchmarked it leaves provisional and its score reflects the measurement. We{" "}
              <b>lead with token efficiency</b>: a skill that measurably <span className="pos">saves tokens</span> rises;
              one that <span className="neg">costs tokens</span> for no success gain drops — even if it has hundreds of
              thousands of stars. Success lift, when a task is hard enough to show it, adds on top.
            </p>
          </div>
        </section>

        <section className="install-panel">
          <p className="glabel">how an eval runs</p>
          <ol className="how-list">
            <li>
              <b>Paired, forced-mode trials.</b> The exact same task is run two ways: <em>control</em> (the agent with
              no skill) and <em>treatment</em> (the agent with the skill loaded). The only variable is the skill.
            </li>
            <li>
              <b>A reproducible suite.</b> Each suite is a fixture repo pinned to a commit + a task instruction + a
              verifier. Everything is content-addressed, so a run is repeatable.
            </li>
            <li>
              <b>Hidden verification.</b> The agent sees the task spec, <em>not</em> the tests. The pass/fail verifier
              is injected only <em>after</em> the agent process exits — so a skill can&apos;t be gamed by peeking at the
              checks, and correctness comes from reasoning.
            </li>
            <li>
              <b>A pinned reference model.</b> Control and treatment use the same agent + model (e.g. Claude Haiku 4.5),
              recorded in the result. Results are only comparable <em>within</em> the same model / agent / suite — never
              mixed across them.
            </li>
            <li>
              <b>N trials per arm.</b> We run several trials each side and report the deltas: <b>token delta</b>{" "}
              (negative = the skill saves tokens) and <b>success delta</b> (percentage points). Low-N results are
              flagged as directional.
            </li>
          </ol>
        </section>

        <section className="detail-grid" aria-label="trust and honesty">
          <div className="pane">
            <p className="glabel">trust tiers</p>
            <p>
              Results carry a tier and are never blended across tiers:
            </p>
            <p className="source-path">
              <b>self-reported</b> — run locally in a worktree (weakest isolation).
            </p>
            <p className="source-path">
              <b>community-reported</b> — run in container isolation, plausibility-checked against the official
              baseline.
            </p>
            <p className="source-path">
              <b>official</b> — audited on skillrank&apos;s reference harness.
            </p>
          </div>
          <div className="pane">
            <p className="glabel">what we won&apos;t do</p>
            <p>
              We don&apos;t fabricate numbers. A skill with no eval shows <b>&quot;pending&quot;</b>, not a made-up
              score. We publish the losers too — if a popular skill costs tokens for no gain, that&apos;s exactly the
              finding worth surfacing. Every eval is reproducible; anyone can re-run it.
            </p>
          </div>
        </section>

        <section className="install-panel" id="reproduce">
          <p className="glabel">reproduce it yourself</p>
          <p>The same eval the registry runs is one CLI command against a public suite:</p>
          <div className="instrow">
            <code>
              <span className="p">$</span> skillrank eval <b>&lt;skill&gt;</b> --suite &lt;id&gt; --model
              claude-haiku-4-5-20251001
            </code>
          </div>
          <p className="dmeta">
            <Link href="/">← back to the registry</Link>
            <a href="https://github.com/buildbetter-app/skillrank">source ▸</a>
          </p>
        </section>
      </main>

      <nav className="keybar" aria-label="commands">
        <Link className="k" href="/">
          <kbd>r</kbd>registry
        </Link>
        <a className="k" href="https://github.com/buildbetter-app/skillrank">
          <kbd>g</kbd>github
        </a>
        <span className="sp" />
      </nav>
    </>
  );
}
