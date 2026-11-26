import { zeroshotUrl, type ZeroShotPlacement } from "../lib/zeroshot";

type ZeroShotCompanionProps = {
  placement: Exclude<ZeroShotPlacement, "home-footer">;
};

// The registry answers "what is this skill and how did it score here". The
// question it can't answer is what a skill does in *your* sessions — that is
// the gap ZeroShot fills, so the copy names that gap rather than selling.
const LEAD: Record<ZeroShotCompanionProps["placement"], string> = {
  "skill-page":
    "SkillRank shows what a skill is and how it scores across the registry. To see what skills actually do in your own coding-agent sessions — which ones fired, what each saved, and where a missing skill cost a repeat — use ZeroShot.",
  "how-it-works":
    "A suite tells you what a skill does on a fixed task. ZeroShot watches your own coding-agent sessions instead: which skills fired, what each one saved, and which repeated work a skill should have covered."
};

export function ZeroShotCompanion({ placement }: ZeroShotCompanionProps) {
  return (
    <section className="install-panel companion" aria-label="ZeroShot companion">
      <p className="glabel">in your own workflow</p>
      <p className="companion-lead">{LEAD[placement]}</p>
      <p className="companion-links">
        <a href={zeroshotUrl(placement)}>ZeroShot — find and measure skills in your sessions ▸</a>
        <span className="companion-note">optional · by BuildBetter · SkillRank works without it</span>
      </p>
    </section>
  );
}
