import type { MetadataRoute } from "next";
import { skills, skillPath } from "../lib/catalog";
import { SITE_URL } from "../lib/site";

/**
 * Every URL here ends in a trailing slash because next.config.mjs sets
 * `trailingSlash: true` — without it each entry would 308-redirect, which
 * wastes crawl budget and weakens the signal. skillPath() already emits one.
 *
 * The 2,000+ skill pages are the whole point of the sitemap: they are the
 * long-tail search surface, and nothing else on the site links to most of them.
 */
export default function sitemap(): MetadataRoute.Sitemap {
  const staticEntries: MetadataRoute.Sitemap = [
    { url: `${SITE_URL}/`, changeFrequency: "daily", priority: 1.0 },
    { url: `${SITE_URL}/how-it-works/`, changeFrequency: "monthly", priority: 0.7 },
  ];

  const skillEntries: MetadataRoute.Sitemap = skills.map((s) => ({
    url: `${SITE_URL}${skillPath(s.slug)}`,
    // added_at is a plain YYYY-MM-DD string; skip anything unparseable rather
    // than emitting an Invalid Date.
    lastModified: s.added_at ? new Date(s.added_at) : undefined,
    changeFrequency: "weekly",
    priority: 0.6,
  }));

  return [...staticEntries, ...skillEntries];
}
