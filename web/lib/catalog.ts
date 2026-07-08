import catalogData from "../data/catalog.json";

export type Skill = {
  slug: string;
  display_name: string;
  category: string;
  tags: string[];
  source_type: "github";
  source_url: string;
  source_repo: string;
  source_subpath: string;
  description: string;
  signals: {
    stars: number | null;
    installs: number | null;
  };
  eval: {
    success_delta_pct: number | null;
    token_delta_pct: number | null;
    n_trials: number | null;
    tier: "official" | "community" | "self";
  };
  score: number;
  provisional: boolean;
  added_at: string;
};

export const SCORE_EXPLAINER =
  "SkillRank score blends community stars, real usage, and our eval lift; provisional until a skill is evaluated -- so popularity alone can't reach the top tier.";

export const skills = catalogData as Skill[];

export function getSkillBySlug(slug: string) {
  return skills.find((skill) => skill.slug === slug);
}

export function skillPath(slug: string) {
  return `/skill/${slug.split("/").map(encodeURIComponent).join("/")}/`;
}

export function formatCount(value: number | null) {
  if (value == null) return "—";
  if (value >= 1000000) return `${(value / 1000000).toFixed(value % 1000000 === 0 ? 0 : 1)}m`;
  if (value >= 1000) return `${(value / 1000).toFixed(value % 1000 === 0 ? 0 : 1)}k`;
  return String(value);
}

export function scoreBand(score: number) {
  if (score >= 85) return "sc-hi";
  if (score >= 60) return "sc-mid";
  return "sc-lo";
}

export function scoreBar(score: number, width = 8) {
  const filled = Math.round((score / 100) * width);
  return `${"█".repeat(filled)}${"░".repeat(width - filled)}`;
}
