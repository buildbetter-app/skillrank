/**
 * The ZeroShot companion link, in one place.
 *
 * SkillRank stays fully usable without ZeroShot; this is the one optional
 * pointer to it. Two rules keep it honest:
 *
 * 1. Link only to the stable distribution. tryzeroshot.com serves the
 *    production channel: on 2026-11-23 its /download/mac and /install.sh both
 *    resolved to the v0.13.0 production assets in
 *    buildbetter-app/zeroshot-releases, never a testing or staging RC. The old
 *    buildbetter.sh domain 308s here, so link it directly and skip the hop.
 * 2. Only claim what the stable landing page claims: session monitoring that
 *    shows which skills fired, what each saved, and where a skill was missing.
 *
 * UTM parameters say which placement sent the visit, so referrals can be
 * attributed on the ZeroShot side without adding client-side tracking here.
 */
export const ZEROSHOT_ORIGIN = "https://tryzeroshot.com/";

export type ZeroShotPlacement = "skill-page" | "how-it-works" | "home-footer";

export function zeroshotUrl(placement: ZeroShotPlacement) {
  const url = new URL(ZEROSHOT_ORIGIN);
  url.searchParams.set("utm_source", "skillrank");
  url.searchParams.set("utm_medium", "referral");
  url.searchParams.set("utm_campaign", "companion");
  url.searchParams.set("utm_content", placement);
  return url.toString();
}
