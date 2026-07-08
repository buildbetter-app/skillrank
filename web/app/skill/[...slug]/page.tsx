import Link from "next/link";
import type { Metadata } from "next";
import { notFound } from "next/navigation";
import {
  SCORE_EXPLAINER,
  formatCount,
  getSkillBySlug,
  scoreBand,
  scoreBar,
  skillPath,
  skills
} from "../../../lib/catalog";

export const dynamicParams = false;

type PageProps = {
  params: Promise<{
    slug: string[];
  }>;
};

function slugFromParams(slug: string[]) {
  return slug.map(decodeURIComponent).join("/");
}

export function generateStaticParams() {
  return skills.map((skill) => ({
    slug: skill.slug.split("/")
  }));
}

export async function generateMetadata({ params }: PageProps): Promise<Metadata> {
  const resolved = await params;
  const skill = getSkillBySlug(slugFromParams(resolved.slug));

  if (!skill) {
    return {
      title: "Skill not found"
    };
  }

  return {
    title: `${skill.display_name} (${skill.slug})`,
    description: `${skill.description} SkillRank score ${skill.score}/100, provisional while evals are pending.`,
    alternates: {
      canonical: skillPath(skill.slug)
    }
  };
}

export default async function SkillPage({ params }: PageProps) {
  const resolved = await params;
  const skill = getSkillBySlug(slugFromParams(resolved.slug));

  if (!skill) {
    notFound();
  }

  return (
    <>
      <header className="topbar">
        <Link className="brand" href="/">
          skillrank<span className="cur">_</span>
        </Link>
        <span className="kv hidesm">
          skill <i>▸</i> {skill.slug}
        </span>
        <span className="sp" />
        <span className="kv">MIT · open registry</span>
      </header>

      <main className="wrap detail-page">
        <section className="skill-hero">
          <p className="tag">skill detail</p>
          <Link href="/" className="backlink">
            ← registry
          </Link>
          <h1>{skill.display_name}</h1>
          <p className="slugline">{skill.slug}</p>
          <p className="lede">{skill.description}</p>
          <div className="detail-tags">
            <span className={`tier ${skill.eval.tier === "official" ? "official" : ""}`}>{skill.eval.tier}</span>
            <span className="badge">provisional</span>
            <span className="tagchip">{skill.category}</span>
            {skill.tags.map((tag) => (
              <span className="tagchip" key={tag}>
                {tag}
              </span>
            ))}
          </div>
        </section>

        <section className="detail-grid" aria-label={`${skill.display_name} signals`}>
          <div className="pane score-pane">
            <p className="glabel">skillrank score</p>
            <div className={`big-score ${scoreBand(skill.score)}`}>
              <span className="bar">{scoreBar(skill.score, 16)}</span>
              <span className="v">{skill.score}</span>
            </div>
            <p className="explain">{SCORE_EXPLAINER}</p>
          </div>

          <div className="pane">
            <p className="glabel">source</p>
            <p className="metric">★ {formatCount(skill.signals.stars)}</p>
            <p className="source-path">{skill.source_repo}</p>
            <p className="source-path">{skill.source_subpath}</p>
            <a href={skill.source_url}>open on GitHub ▸</a>
          </div>

          <div className="pane pending-pane">
            <p className="glabel">eval status</p>
            <p className="pending-title">eval pending</p>
            <p>
              Success delta, token delta, and trial count are not available yet. No eval number is shown until this
              skill has measured results.
            </p>
          </div>
        </section>

        <section className="install-panel" id="install">
          <p className="glabel">install</p>
          <div className="instrow">
            <code>
              <span className="p">$</span> skillrank install <b>{skill.slug}</b>
            </code>
          </div>
          <div className="installrow secondary-install">
            <code>
              <span className="p">$</span> curl -fsSL <b>skillrank.dev</b> | sh
            </code>
          </div>
        </section>
      </main>

      <nav className="keybar" aria-label="commands">
        <Link className="k" href="/">
          <kbd>r</kbd>registry
        </Link>
        <a className="k" href={skill.source_url}>
          <kbd>g</kbd>github
        </a>
        <a className="k" href="#install">
          <kbd>i</kbd>install
        </a>
        <span className="sp" />
      </nav>
    </>
  );
}
