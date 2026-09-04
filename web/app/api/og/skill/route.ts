import { createElement } from "react";
import { ImageResponse } from "next/og";
import { getSkillBySlug } from "../../../../lib/catalog";
import { SkillSocialCard } from "./social-card";

export const runtime = "nodejs";

export async function GET(request: Request) {
  const slug = new URL(request.url).searchParams.get("slug");
  const skill = slug ? getSkillBySlug(slug) : undefined;

  if (!skill) {
    return new Response("Skill not found", { status: 404 });
  }

  return new ImageResponse(createElement(SkillSocialCard, { skill }), {
    width: 1200,
    height: 630,
    headers: {
      "Cache-Control": "public, max-age=3600, s-maxage=86400, stale-while-revalidate=604800"
    }
  });
}
