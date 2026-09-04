import type { Skill } from "../../../../lib/catalog";

function compact(text: string, maxLength: number) {
  if (text.length <= maxLength) return text;
  return `${text.slice(0, maxLength - 1).trimEnd()}…`;
}

export function SkillSocialCard({ skill }: { skill: Skill }) {
  const titleSize = skill.display_name.length > 30 ? 62 : skill.display_name.length > 21 ? 72 : 82;
  const tags = [skill.category, ...skill.tags].slice(0, 3);

  return (
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        position: "relative",
        overflow: "hidden",
        color: "#f6f7ef",
        background: "#070a09",
        fontFamily: "ui-sans-serif, system-ui, sans-serif"
      }}
    >
      <div
        style={{
          position: "absolute",
          inset: 0,
          display: "flex",
          backgroundImage:
            "linear-gradient(rgba(190,255,0,0.035) 1px, transparent 1px), linear-gradient(90deg, rgba(190,255,0,0.035) 1px, transparent 1px)",
          backgroundSize: "42px 42px"
        }}
      />
      <div
        style={{
          position: "absolute",
          width: 620,
          height: 620,
          right: -210,
          top: -250,
          display: "flex",
          borderRadius: 620,
          background: "radial-gradient(circle, rgba(190,255,0,0.16), rgba(190,255,0,0) 68%)"
        }}
      />

      <div
        style={{
          width: "100%",
          height: "100%",
          padding: "58px 64px 54px",
          display: "flex",
          flexDirection: "column",
          justifyContent: "space-between"
        }}
      >
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              color: "#beff00",
              fontSize: 20,
              fontWeight: 800,
              letterSpacing: "0.14em"
            }}
          >
            SKILL SPOTLIGHT
          </div>
          <div style={{ display: "flex", alignItems: "center", fontSize: 30, fontWeight: 800 }}>
            skillrank<span style={{ color: "#beff00" }}>_</span>
          </div>
        </div>

        <div style={{ display: "flex", flexDirection: "column", width: 820 }}>
          <div
            style={{
              display: "flex",
              color: "#f6f7ef",
              fontSize: titleSize,
              lineHeight: 0.98,
              fontWeight: 900,
              letterSpacing: "-0.045em"
            }}
          >
            {compact(skill.display_name, 42)}
          </div>
          <div
            style={{
              display: "flex",
              marginTop: 25,
              color: "#c5cabf",
              fontSize: 27,
              lineHeight: 1.3,
              fontWeight: 500
            }}
          >
            {compact(skill.description, 138)}
          </div>
          <div style={{ display: "flex", marginTop: 24 }}>
            {tags.map((tag) => (
              <div
                key={tag}
                style={{
                  display: "flex",
                  marginRight: 12,
                  padding: "8px 13px",
                  border: "1px solid rgba(190,255,0,0.34)",
                  borderRadius: 999,
                  color: "#d8f58a",
                  background: "rgba(190,255,0,0.055)",
                  fontSize: 16,
                  fontWeight: 700,
                  letterSpacing: "0.04em"
                }}
              >
                {compact(tag, 22)}
              </div>
            ))}
          </div>
        </div>

        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            padding: "18px 22px",
            border: "1px solid rgba(190,255,0,0.54)",
            borderRadius: 15,
            background: "rgba(4, 7, 6, 0.82)"
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
              fontSize: 24,
              fontWeight: 700
            }}
          >
            <span style={{ color: "#beff00", marginRight: 14 }}>&gt;_</span>
            skillrank install {skill.slug}
          </div>
          <div style={{ display: "flex", color: "#f7bd3e", fontSize: 17, fontWeight: 800 }}>
            SKILLRANK.DEV
          </div>
        </div>
      </div>
    </div>
  );
}
