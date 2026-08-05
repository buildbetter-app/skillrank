// Hosted SkillRank registry — serves the /v3/rest/skill-registry contract from
// the ingested public-skill catalog, plus the account-gated write side.
//
//   search / show  -> over ALL catalog entries (enriched.json)
//   resolve        -> installable entries carry pinned commit + content + hash
//                     (ingested.json); collections resolve to a tombstone
//                     pointing at the source repo.
//   auth / publish -> tokens and eval-result ingest, backed by Upstash
//                     (registry/lib/*). Reads stay fully anonymous and do not
//                     vary when a token is present.
//
// Every READ route is anonymous and edge-cacheable, and answers identically
// whether or not a token is presented. The write routes are account-gated and
// always `no-store`.
//
// Content hashes were computed by the ingestion pipeline the SAME way as the Rust
// client (skillrank-core::hash), so `skillrank install` hash-verification passes.

import { readFileSync } from "node:fs";
import { Redis } from "@upstash/redis";

import {
  githubConfig,
  githubDeviceStart,
  githubExchange,
  isVerifiedKind,
  createAnonymousAccount,
  parseBearer,
  parseBearerRaw,
  revokeAccount,
  revokeToken,
  secretsEqual,
  upsertVerifiedAccount,
  verifyToken,
} from "../lib/auth.mjs";
import { flagAccountResults, ingestBundle, ingestConfig, readSkillCells } from "../lib/ingest.mjs";
import { normalizeTier } from "../lib/scan.mjs";
import { clientIpBucket, createStore, rateLimits } from "../lib/store.mjs";

const enriched = JSON.parse(readFileSync(new URL("./enriched.json", import.meta.url), "utf8"));
const ingested = JSON.parse(readFileSync(new URL("./ingested.json", import.meta.url), "utf8"));
// NOTE: use LITERAL new URL(...) args (not a variable path) so Vercel's file
// tracer bundles these JSON files into the deployed function.
function safeRead(url, fallback) {
  try {
    return JSON.parse(readFileSync(url, "utf8"));
  } catch {
    return fallback;
  }
}
const suites = safeRead(new URL("./suites.json", import.meta.url), []);
const verifiers = safeRead(new URL("./verifiers.json", import.meta.url), {});

const bySlug = new Map(enriched.map((e) => [e.slug, e]));
const installBySlug = new Map(ingested.map((e) => [e.slug, e]));
const sorted = [...enriched].sort((a, b) => (b.score || 0) - (a.score || 0));

/// The tier the ingestion pipeline computed from the pinned content.
///
/// This used to return a hardcoded `"pending"` for everything installable, which
/// meant nothing the pipeline wrote was ever served: 188 of 200 skills came back
/// `pending` and 12 `unknown`, so the client treated 100% of the catalog as
/// unverified and prompted on every install.
///
/// Entries with no `scan_tier` are ones whose content was never fetched
/// (collections, unreachable repos) or that were pinned before the scanner
/// existed — `unknown` and `pending` respectively, which is the truth in both
/// cases. The value is normalized because `scan_tier` deserializes into a Rust
/// enum on the client: an unrecognized string would fail the whole response, so
/// a bad value degrades to `unknown` instead of breaking the caller.
function scanTier(e) {
  if (!e) return "unknown";
  // `enriched.json` marks this with `status`; `ingested.json` rows carry
  // `installable` instead, and both flow through here.
  const fallback = e.status === "installable" || e.installable === true ? "pending" : "unknown";
  if (typeof e.scan_tier === "string" && e.scan_tier) return normalizeTier(e.scan_tier, fallback);
  return fallback;
}

/// Findings behind a non-safe tier, shaped for a confirmation dialog.
///
/// ADDITIVE ONLY. The compiled Rust `SkillDetail` / `ResolveResponse` must keep
/// deserializing, so this is a new optional object under a new key and no
/// existing field changes shape. Omitted entirely when there is nothing to say,
/// so `safe` responses do not grow.
function scanReport(e) {
  const scan = e && e.scan;
  if (!scan || typeof scan !== "object") return null;
  const findings = Array.isArray(scan.findings) ? scan.findings : [];
  return {
    tier: scanTier(e),
    score: typeof scan.score === "number" ? scan.score : 0,
    scanner_version: typeof scan.scanner_version === "string" ? scan.scanner_version : "",
    findings: findings.slice(0, 16).map((f) => ({
      rule: String(f.rule || ""),
      severity: String(f.severity || ""),
      line: Number.isFinite(f.line) ? f.line : 0,
      excerpt: String(f.excerpt || ""),
      why: String(f.why || ""),
    })),
  };
}

function summary(e) {
  return {
    slug: e.slug,
    display_name: e.display_name,
    category: e.category || "",
    stacks: e.tags || [],
    source_type: "github",
    source_url: e.source_url || "",
    latest_version: e.content_hash || "",
    scan_tier: scanTier(e),
    signals_score: typeof e.score === "number" ? e.score : null,
    rating_count: 0,
    summary: e.description || "",
  };
}

/// One row of the eval-suite index. Task INSTRUCTIONS are the bulk of a suite and
/// a caller choosing between suites cannot use them, so the list carries only what
/// a picker needs — id, version, size, and the reference env a run must match —
/// and `GET /eval-suites/:id` still serves the full definition.
function suiteSummary(s) {
  const out = {
    id: s.id || "",
    version: s.version || "",
    task_count: Array.isArray(s.tasks) ? s.tasks.length : 0,
  };
  // Omitted rather than blanked when a suite has no title/description, so a
  // client can tell "not provided" from "provided and empty".
  if (s.title) out.title = String(s.title);
  if (s.description) out.description = String(s.description);
  if (s.reference_env) out.reference_env = s.reference_env;
  return out;
}

/// Count how many catalog entries carry each value of one facet.
///
/// Values are normalized exactly the way the `/skills` filters compare them
/// (trimmed, lowercased) and de-duplicated per entry, so a `count` here is
/// precisely the `total` that filtering on that value returns — a client that
/// renders these as filter options can never offer one that matches nothing.
/// Ties break alphabetically so the order is stable across deploys.
function countFacet(pick) {
  const counts = new Map();
  for (const e of enriched) {
    const seen = new Set();
    for (const raw of pick(e)) {
      const value = String(raw ?? "").trim().toLowerCase();
      if (!value || seen.has(value)) continue;
      seen.add(value);
      counts.set(value, (counts.get(value) || 0) + 1);
    }
  }
  return [...counts.entries()]
    .map(([value, count]) => ({ value, count }))
    .sort((a, b) => b.count - a.count || (a.value < b.value ? -1 : 1));
}

// The catalog is a static import, so the taxonomy is a constant: compute it once
// at module load and let the route just serialize it.
const facets = {
  categories: countFacet((e) => [e.category]),
  stacks: countFacet((e) => e.tags || []),
  scan_tiers: countFacet((e) => [scanTier(e)]),
};

const stripSep = (s) => s.replace(/[\s\-_/]/g, "");
function matchesQuery(e, q) {
  const hay = [e.slug, e.display_name, e.summary || e.description, e.category, (e.tags || []).join(" "), e.source_repo]
    .join(" ")
    .toLowerCase();
  const collapsed = stripSep(q);
  if (collapsed && stripSep(hay).includes(collapsed)) return true;
  const words = q.split(/\s+/).filter(Boolean);
  return words.length > 0 && words.every((w) => hay.includes(w));
}

function json(res, status, body, cache = "public, s-maxage=60, stale-while-revalidate=600") {
  res.statusCode = status;
  res.setHeader("Content-Type", "application/json; charset=utf-8");
  res.setHeader("Cache-Control", cache);
  res.end(JSON.stringify(body));
}

// ---- optional datastore (Upstash Redis) ---------------------------------
// Everything below degrades to a NO-OP when the store is not provisioned, so
// the read-side registry keeps working with zero env vars. We only ever store
// aggregate counters and emails the user explicitly typed — never IPs or PII.
let _redis;
let _redisInit = false;
function redis() {
  if (_redisInit) return _redis;
  _redisInit = true;
  try {
    const url = process.env.KV_REST_API_URL || process.env.UPSTASH_REDIS_REST_URL;
    const token = process.env.KV_REST_API_TOKEN || process.env.UPSTASH_REDIS_REST_TOKEN;
    if (url && token) _redis = new Redis({ url, token });
  } catch {
    _redis = undefined;
  }
  return _redis;
}

async function bumpInstall(slug) {
  const r = redis();
  if (!r) return;
  try {
    await r.zincrby("installs", 1, slug);
  } catch {
    /* telemetry must never break a resolve */
  }
}

function readBody(req) {
  return new Promise((resolve) => {
    let data = "";
    req.on("data", (c) => {
      data += c;
      if (data.length > 8192) req.destroy(); // cap body size
    });
    req.on("end", () => resolve(data));
    req.on("error", () => resolve(""));
  });
}

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

// ---- write side: accounts, tokens, eval ingest ---------------------------
// Unlike the counters above, these MUST fail loudly when the store is missing:
// answering 200 to a publish we cannot persist would be worse than an outage.

let _store;
let _storeInit = false;
function store() {
  if (_storeInit) return _store;
  _storeInit = true;
  _store = createStore(redis());
  return _store;
}

/// What ingest is allowed to know about the catalog. `bySlug` covers every entry
/// (including collections, which have no installable hash and are therefore
/// rejected at the hash check); `installBySlug` holds the published hash.
const catalog = {
  hasSkill: (slug) => bySlug.has(slug),
  publishedHash: (slug) => (installBySlug.get(slug) || {}).content_hash || "",
};

const ACCOUNT_ID_RE = /^acct_[A-Za-z0-9_-]{8,64}$/;
// Vercel refuses request bodies above ~4.5 MB upstream, so this ceiling only
// exists so a rogue client cannot make us buffer without bound.
const HARD_BODY_CAP = 4 * 1024 * 1024;

/// Body reader for the write routes. The `readBody` above destroys the socket
/// past 8 KiB, which never settles for a large POST; this one keeps draining so
/// `end` fires and the caller can answer with a real 413.
///
/// Nothing here destroys the request. Doing that at the hard ceiling is what
/// turned the 413 into an ECONNRESET/EPIPE: the socket was already gone by the
/// time the route wrote its answer, so the caller saw "registry unreachable"
/// instead of "bundle exceeds N bytes; publish fewer trials". Past the ceiling
/// we stop waiting for `end` and settle immediately so the route can answer at
/// once; buffering already stopped at `maxBytes`, so a huge body costs bandwidth
/// but not memory.
function readBodyLimited(req, maxBytes) {
  return new Promise((resolve) => {
    const chunks = [];
    let size = 0;
    let overflow = false;
    let settled = false;
    const settle = (value) => {
      if (settled) return;
      settled = true;
      resolve(value);
    };
    req.on("data", (c) => {
      const buf = Buffer.isBuffer(c) ? c : Buffer.from(c);
      size += buf.length;
      if (size > maxBytes) {
        overflow = true;
        if (size > HARD_BODY_CAP) settle({ text: "", size, overflow: true, aborted: false });
        return;
      }
      chunks.push(buf);
    });
    req.on("end", () => settle({ text: Buffer.concat(chunks).toString("utf8"), size, overflow, aborted: false }));
    req.on("error", () => settle({ text: "", size, overflow, aborted: true }));
    req.on("aborted", () => settle({ text: "", size, overflow, aborted: true }));
  });
}

/// True once the response can no longer be written. Checking `res.destroyed`
/// alone is not enough: when the peer goes away, `res.socket.destroyed` flips
/// immediately while `res.destroyed` stays false for tens of milliseconds, so
/// the guard read "still writable" exactly when it was not.
function connectionGone(res) {
  return Boolean(res.writableEnded || res.destroyed || (res.socket && res.socket.destroyed));
}

/// Rate-limit bucket for the caller's IP. `clientIpBucket` (in `lib/store.mjs`,
/// with the salt policy that makes the persisted key name non-invertible) is a
/// pure function of the environment, so it is passed `process.env` here.
const ipBucketOf = (req) => clientIpBucket(req, process.env);

/// Returns true when the request was rejected (and the response already sent).
async function rateLimited(res, s, bucket, max, windowSeconds) {
  const hit = await s.hit(bucket, max, windowSeconds);
  if (hit.allowed) return false;
  // Must be an integer delta-seconds header: the CLI prints "retry after N seconds".
  res.setHeader("Retry-After", String(hit.retryAfter));
  const reason = `too many requests; retry in ${hit.retryAfter} seconds`;
  json(res, 429, { accepted: false, error: "rate limited", reason }, "no-store");
  return true;
}

function unauthorized(res, reason) {
  return json(res, 401, { accepted: false, error: "not signed in", reason }, "no-store");
}

function storeUnavailable(res, what) {
  const reason = `the registry datastore is not available, so ${what} is disabled`;
  return json(res, 503, { accepted: false, error: reason, reason }, "no-store");
}

async function readJsonBody(req, maxBytes) {
  const body = await readBodyLimited(req, maxBytes);
  if (body.overflow || body.aborted) return { ok: false, tooLarge: body.overflow };
  try {
    return { ok: true, value: JSON.parse(body.text || "null") };
  } catch {
    return { ok: false, tooLarge: false };
  }
}

// ---- accounts: /auth/... -------------------------------------------------
async function handleAuth(req, res, parts) {
  const route = parts[0] || "";
  const limits = rateLimits(process.env);
  const s = store();
  const nowIso = new Date().toISOString();

  // GET /auth/whoami — token introspection. Never 401s: "who am I" with no
  // credential has a true answer, and `skillrank whoami` should print it.
  if (route === "whoami") {
    if (req.method !== "GET") return json(res, 405, { error: "method not allowed" }, "no-store");
    const anonymousAnswer = { authenticated: false, kind: "", account_id: "", created_at: "", verified: false };
    const token = parseBearer(req.headers.authorization);
    if (!token) return json(res, 200, anonymousAnswer, "no-store");
    if (!s) return storeUnavailable(res, "sign-in");
    if (await rateLimited(res, s, `whoami:${ipBucketOf(req)}`, limits.whoamiPerIpPerHour, 3600)) return;
    const verified = await verifyToken(s, token);
    if (!verified.ok) return json(res, 200, anonymousAnswer, "no-store");
    return json(
      res,
      200,
      {
        authenticated: true,
        kind: verified.account.kind,
        account_id: verified.account.account_id,
        created_at: verified.account.created_at || "",
        verified: isVerifiedKind(verified.account.kind),
      },
      "no-store",
    );
  }

  // POST /auth/device — start a GitHub device authorization for the CLI.
  if (route === "device") {
    if (req.method !== "POST") return json(res, 405, { error: "method not allowed" }, "no-store");
    if (!githubConfig(process.env).enabled) {
      return json(res, 503, { error: "github sign-in is not configured on this registry" }, "no-store");
    }
    if (!s) return storeUnavailable(res, "sign-in");
    if (await rateLimited(res, s, `device:${ipBucketOf(req)}`, limits.deviceStartsPerIpPerHour, 3600)) return;
    const started = await githubDeviceStart(process.env);
    if (!started.ok) return json(res, 502, { error: started.reason }, "no-store");
    return json(
      res,
      200,
      {
        device_code: started.device_code,
        user_code: started.user_code,
        verification_uri: started.verification_uri,
        interval: started.interval,
        expires_in: started.expires_in,
      },
      "no-store",
    );
  }

  // POST /auth/tokens — mint. DELETE /auth/tokens — revoke the presented token.
  if (route === "tokens") {
    if (req.method === "DELETE") {
      const token = parseBearer(req.headers.authorization);
      if (!token) return unauthorized(res, "no token presented");
      if (!s) return storeUnavailable(res, "revocation");
      const revoked = await revokeToken(s, token, nowIso);
      if (!revoked) return unauthorized(res, "token is not valid");
      return json(res, 200, { revoked: true }, "no-store");
    }
    if (req.method !== "POST") return json(res, 405, { error: "method not allowed" }, "no-store");
    if (!s) return storeUnavailable(res, "token issuance");

    const body = await readJsonBody(req, 8192);
    if (!body.ok) {
      if (connectionGone(res)) return undefined;
      return json(res, 400, { error: "body must be a small JSON object" }, "no-store");
    }
    const kind = typeof body.value?.kind === "string" ? body.value.kind.trim() : "";

    // Anonymous tokens exist so ZeroShot can provision one on install without a
    // copy-paste. They publish, but they are never independent (see R5), which is
    // exactly why minting one freely is safe.
    if (kind === "anonymous") {
      const bucket = `anon:${ipBucketOf(req)}`;
      if (await rateLimited(res, s, bucket, limits.anonTokensPerIpPerDay, 86400)) return;
      const { account, token } = await createAnonymousAccount(s, nowIso);
      return json(
        res,
        201,
        {
          token,
          kind: account.kind,
          account_id: account.account_id,
          created_at: account.created_at,
          verified: false,
          note: "anonymous results publish as self-reported and never count toward community corroboration",
        },
        "no-store",
      );
    }

    if (kind === "github") {
      if (!githubConfig(process.env).enabled) {
        return json(res, 503, { error: "github sign-in is not configured on this registry" }, "no-store");
      }
      const code = body.value?.device_code || body.value?.code;
      if (typeof code !== "string" || !code || code.length > 256) {
        return json(res, 400, { error: "device_code is required" }, "no-store");
      }
      if (await rateLimited(res, s, `ghpoll:${ipBucketOf(req)}`, limits.githubPollsPerIpPerHour, 3600)) return;

      const exchange = await githubExchange(process.env, code);
      if (exchange.state === "pending" || exchange.state === "slow_down") {
        return json(res, 202, { status: exchange.state, interval: exchange.interval }, "no-store");
      }
      if (exchange.state === "unconfigured") return json(res, 503, { error: exchange.reason }, "no-store");
      if (exchange.state === "failed") return json(res, 502, { error: exchange.reason }, "no-store");
      if (exchange.state !== "ok") return json(res, 400, { error: exchange.reason }, "no-store");

      const result = await upsertVerifiedAccount(s, { provider: "github", subject: exchange.subject, now: nowIso });
      // Another invocation claimed this identity's pointer but its account record
      // is not readable yet. Minting a second account here is what would make one
      // GitHub user into N "independent" corroborations, so retry instead.
      if (result.conflict) {
        return json(res, 503, { error: "sign-in is briefly unavailable for this account; try again" }, "no-store");
      }
      if (result.revoked) return json(res, 403, { error: "this account has been revoked" }, "no-store");
      return json(
        res,
        201,
        {
          token: result.token,
          kind: result.account.kind,
          account_id: result.account.account_id,
          created_at: result.account.created_at,
          verified: true,
        },
        "no-store",
      );
    }

    return json(res, 400, { error: 'kind must be "anonymous" or "github"' }, "no-store");
  }

  // POST /auth/accounts/:id/revoke — maintainer abuse response (R6). Kills the
  // account's tokens and flags everything it published for review.
  if (route === "accounts" && parts[2] === "revoke") {
    if (req.method !== "POST") return json(res, 405, { error: "method not allowed" }, "no-store");
    const adminToken = (process.env.REGISTRY_ADMIN_TOKEN || "").trim();
    if (!adminToken) return json(res, 503, { error: "maintainer actions are not configured" }, "no-store");
    // Compared raw and in constant time. `parseBearer` applies the charset of the
    // tokens WE mint, which silently reduced any password-manager credential
    // (`Xk9#mQ2$…`) to "" and produced a permanent 401 indistinguishable from a
    // wrong secret — during the one incident where revocation is the lever.
    const presented = parseBearerRaw(req.headers.authorization);
    if (!presented || !secretsEqual(presented, adminToken)) return unauthorized(res, "maintainer credential required");
    if (!s) return storeUnavailable(res, "revocation");
    const accountId = parts[1] || "";
    if (!ACCOUNT_ID_RE.test(accountId)) return json(res, 400, { error: "malformed account id" }, "no-store");
    const account = await revokeAccount(s, accountId, nowIso);
    if (!account) return json(res, 404, { error: "not found" }, "no-store");
    const flagged = await flagAccountResults(s, accountId, { config: ingestConfig(process.env), now: new Date() });
    return json(res, 200, { account_id: accountId, revoked_at: account.revoked_at, ...flagged }, "no-store");
  }

  return json(res, 404, { error: "not found" }, "no-store");
}

// ---- eval ingest: POST /eval-results -------------------------------------
async function handleEvalResults(req, res) {
  if (req.method !== "POST") {
    return json(res, 405, { accepted: false, error: "method not allowed", reason: "use POST" }, "no-store");
  }
  const config = ingestConfig(process.env);
  const limits = rateLimits(process.env);

  // Drain first: Vercel has already received the body, and answering before the
  // stream ends turns a clean 4xx into a transport error for some clients.
  const body = await readBodyLimited(req, config.maxBodyBytes);
  if (connectionGone(res)) return undefined;
  if (body.overflow) {
    const reason = `bundle exceeds ${config.maxBodyBytes} bytes; publish fewer trials`;
    return json(res, 413, { accepted: false, error: reason, reason }, "no-store");
  }
  if (body.aborted) {
    const reason = "the request body was truncated; retry the publish";
    return json(res, 400, { accepted: false, error: reason, reason }, "no-store");
  }

  const token = parseBearer(req.headers.authorization);
  if (!token) return unauthorized(res, "publishing requires an account; run `skillrank login` first");

  const s = store();
  if (!s) return storeUnavailable(res, "publishing");

  const ipBucket = ipBucketOf(req);
  if (await rateLimited(res, s, `pub:ip:${ipBucket}`, limits.publishesPerIpPerHour, 3600)) return;

  const verified = await verifyToken(s, token);
  if (!verified.ok) return unauthorized(res, "this token is not valid; run `skillrank login` again");
  const accountId = verified.account.account_id;
  if (await rateLimited(res, s, `pub:acct:h:${accountId}`, limits.publishesPerAccountPerHour, 3600)) return;
  if (await rateLimited(res, s, `pub:acct:d:${accountId}`, limits.publishesPerAccountPerDay, 86400)) return;

  let bundle;
  try {
    bundle = JSON.parse(body.text || "null");
  } catch {
    const reason = "body is not valid JSON";
    return json(res, 400, { accepted: false, error: reason, reason }, "no-store");
  }

  const result = await ingestBundle({
    store: s,
    bundle,
    account: { ...verified.account, verified: isVerifiedKind(verified.account.kind) },
    suites,
    catalog,
    config,
    now: new Date(),
  });
  return json(res, result.status, result.body, "no-store");
}

export default async function handler(req, res) {
  try {
    return await route(req, res);
  } catch (err) {
    // This endpoint takes bearer tokens, so nothing about the request is echoed
    // and no stack is rendered — only a message for the platform log.
    console.error("registry: unhandled error:", err && err.message);
    if (res.headersSent) return undefined;
    return json(res, 500, { accepted: false, error: "internal error", reason: "internal error" }, "no-store");
  }
}

async function route(req, res) {
  const url = new URL(req.url, "http://x");
  const path = url.searchParams.get("path") || "";
  const parts = path.split("/").filter(Boolean);

  // ---- email capture: POST /subscribe { email } ----
  if (parts[0] === "subscribe") {
    if (req.method !== "POST") return json(res, 405, { error: "method not allowed" }, "no-store");
    const raw = await readBody(req);
    let email = "";
    try {
      email = String(JSON.parse(raw || "{}").email || "").trim().toLowerCase();
    } catch {
      email = "";
    }
    if (!EMAIL_RE.test(email) || email.length > 254) {
      return json(res, 400, { ok: false, error: "invalid email" }, "no-store");
    }
    const r = redis();
    let stored = false;
    if (r) {
      try {
        await r.sadd("emails", email);
        stored = true;
      } catch {
        stored = false;
      }
    }
    return json(res, 200, { ok: stored, stored }, "no-store");
  }

  // ---- popularity: GET /installs?limit=N -> top slugs by install-intent ----
  if (parts[0] === "installs") {
    const limit = Math.max(1, Math.min(100, parseInt(url.searchParams.get("limit") || "20", 10) || 20));
    const r = redis();
    let items = [];
    if (r) {
      try {
        const raw = await r.zrange("installs", 0, limit - 1, { rev: true, withScores: true });
        for (let i = 0; i < raw.length; i += 2) items.push({ slug: raw[i], count: Number(raw[i + 1]) });
      } catch {
        items = [];
      }
    }
    return json(res, 200, { items }, "public, s-maxage=30");
  }

  // ---- eval suites: /eval-suites, /eval-suites/:id, /eval-suites/:id/verifiers ----
  if (parts[0] === "eval-suites") {
    const id = parts[1] || "";
    // The index. Without it a suite id can only be typed from memory, which is
    // not a discoverable contract; it is a plain anonymous read like its
    // neighbours, and deliberately omits task bodies.
    if (!id) return json(res, 200, { items: suites.map(suiteSummary), total: suites.length });
    const suite = suites.find((s) => s.id === id);
    if (!suite) return json(res, 404, { error: "suite not found" });
    if (parts[2] === "verifiers") {
      return json(res, 200, verifiers[id] || {});
    }
    return json(res, 200, suite);
  }

  // ---- accounts + publishing (write side; never edge-cached) ----
  if (parts[0] === "auth") return handleAuth(req, res, parts.slice(1));
  if (parts[0] === "eval-results") return handleEvalResults(req, res);

  if (parts[0] !== "skills") return json(res, 404, { error: "not found" });
  const rest = parts.slice(1);

  // /skills -> search
  if (rest.length === 0) {
    const q = (url.searchParams.get("q") || "").toLowerCase();
    const stack = (url.searchParams.get("stack") || "").toLowerCase();
    const category = (url.searchParams.get("category") || "").toLowerCase();
    // `/skills/facets` advertises scan tiers as a filter vocabulary, so this has
    // to honour them: without it, picking a tier facet silently returned the
    // whole catalog instead of the count the facet promised. Compared through
    // `scanTier` rather than the raw field so the same normalization (and the
    // pending/unknown fallback) applies on both sides.
    const scan = (url.searchParams.get("scan_tier") || "").toLowerCase();
    const limit = Math.max(1, parseInt(url.searchParams.get("limit") || "20", 10) || 20);
    let items = sorted.filter((e) => {
      if (stack && !(e.tags || []).some((s) => s.toLowerCase() === stack)) return false;
      if (category && (e.category || "").toLowerCase() !== category) return false;
      if (scan && scanTier(e) !== scan) return false;
      if (q && !matchesQuery(e, q)) return false;
      return true;
    });
    const total = items.length;
    items = items.slice(0, limit).map(summary);
    return json(res, 200, { items, total });
  }

  // /skills/facets -> the catalog's real filter vocabulary, with counts.
  //
  // Must be answered BEFORE the slug parse below, which joins every remaining
  // segment into a slug: `facets` would otherwise be looked up as a skill and
  // 404. Reserving it costs nothing — catalog slugs are always `owner/name`, so
  // no single-segment path can ever name one.
  if (rest.length === 1 && rest[0] === "facets") {
    return json(res, 200, facets);
  }

  // /skills/<slug...>/resolve  or  /skills/<slug...>/eval-results  or  /skills/<slug...>
  const last = rest[rest.length - 1];
  const isResolve = last === "resolve";
  const isEvalResults = last === "eval-results";
  const slug = (isResolve || isEvalResults ? rest.slice(0, -1) : rest).join("/");
  const e = bySlug.get(slug);
  if (!e) return json(res, 404, { error: "not found" });

  // Published eval evidence, read back anonymously. Contributor identities are
  // never in this payload — only counts — and the response does not vary with a
  // token, so it stays edge-cacheable like the other reads.
  if (isEvalResults) {
    const s = store();
    if (!s) return json(res, 200, { slug, cells: [] });
    try {
      return json(res, 200, { slug, cells: await readSkillCells(s, slug) });
    } catch {
      // Rendering "no evidence" during a store outage would misstate a trust
      // surface, so say so instead of quietly returning an empty list.
      return json(res, 503, { error: "eval results are temporarily unavailable" }, "no-store");
    }
  }

  if (isResolve) {
    const inst = installBySlug.get(slug);
    if (!inst) {
      // a collection / non-single-SKILL.md source
      return json(res, 200, {
        slug: e.slug,
        version: "",
        source_type: "github",
        source_url: e.source_url || "",
        content_hash: "",
        // No SKILL.md exists to scan, so there is genuinely no verdict.
        scan_tier: scanTier(e),
        tombstoned: true,
        tombstone_reason: `"${e.slug}" is a skill collection, not a single SKILL.md. Browse and install from the source repo: ${e.source_url}`,
      });
    }
    // The CLI calls resolve immediately before every install, so this is our
    // best server-side install-intent signal. Count it (best-effort) and skip
    // CDN caching on this response so counts aren't hidden behind the edge cache.
    await bumpInstall(slug);
    // `install` is the one place a tier changes what the user sees, so resolve
    // carries the evidence too: the CLI/ZeroShot prompt can name the exact line
    // that made a skill `medium` instead of saying "unverified".
    const report = scanReport(inst);
    return json(
      res,
      200,
      {
        slug: inst.slug,
        version: inst.content_hash,
        source_type: "github",
        source_url: inst.source_url || "",
        source_subpath: inst.skill_path || inst.source_subpath || "",
        pinned_commit: inst.pinned_commit || "",
        content_hash: inst.content_hash,
        scan_tier: scanTier(inst),
        signals_score: typeof inst.score === "number" ? inst.score : null,
        raw_content_url: inst.raw_content_url || "",
        tombstoned: false,
        ...(report ? { scan: report } : {}),
      },
      "no-store",
    );
  }

  // show
  const inst = installBySlug.get(slug);
  const report = scanReport(inst);
  return json(res, 200, {
    ...summary(e),
    versions: inst
      ? [
          {
            content_hash: inst.content_hash,
            pinned_commit: inst.pinned_commit || "",
            scan_tier: scanTier(inst),
            published_at: e.signals?.pushed_at || "",
          },
        ]
      : [],
    eval_cells: [],
    ...(report ? { scan: report } : {}),
  });
}
