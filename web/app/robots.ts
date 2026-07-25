import type { MetadataRoute } from "next";
import { SITE_URL } from "../lib/site";

/**
 * skillrank.dev served no robots.txt at all — /robots.txt returned the app's
 * 404 page. robots.txt is only valid for the host that serves it, and it is
 * what points crawlers (including the AI ones that index skill registries) at
 * the sitemap.
 *
 * /install.sh is disallowed: middleware.ts serves the installer script from
 * `/` to non-browser user agents, and the script itself has nothing to index.
 */
export default function robots(): MetadataRoute.Robots {
  return {
    rules: [{ userAgent: "*", allow: "/", disallow: ["/api/", "/install.sh"] }],
    sitemap: `${SITE_URL}/sitemap.xml`,
    host: SITE_URL,
  };
}
