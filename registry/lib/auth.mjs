// Registry accounts and bearer tokens (spec R1/R2).
//
// Two account classes exist, and the difference is the whole point:
//
//   anonymous — self-service, no identity. ZeroShot provisions one silently on
//               install so nobody has to copy-paste a token. These accounts can
//               publish, and their results are Self-reported forever. They never
//               count toward Community independence, because minting one is free
//               and "N independent accounts" would otherwise mean "N installs".
//   github    — bound to a GitHub user id via the OAuth device flow. Creating N
//               of these has real cost, so only these corroborate (spec R5).
//
// Tokens are 256 bits of `randomBytes`, returned in plaintext exactly once, and
// stored only as a SHA-256 hash. No pepper/HMAC: the token is already uniformly
// random over 2^256, so an offline attacker who steals the hash store has nothing
// to brute-force, and a pepper would silently invalidate every issued token the
// day it were rotated. Nothing here ever logs a token.

import { createHash, randomBytes, timingSafeEqual } from "node:crypto";

import { KEYS } from "./store.mjs";

export const TOKEN_PREFIX = "srk_";
export const KIND_ANONYMOUS = "anonymous";
export const KIND_GITHUB = "github";

const GITHUB_DEVICE_CODE_URL = "https://github.com/login/device/code";
const GITHUB_TOKEN_URL = "https://github.com/login/oauth/access_token";
const GITHUB_USER_URL = "https://api.github.com/user";
const GITHUB_TIMEOUT_MS = 8000;

/// Conservative: everything a REGISTRY-MINTED bearer token may contain. Anything
/// else is rejected before it is hashed or used as key material. This charset is
/// a property of `mintTokenValue`, so it must never be applied to a credential we
/// did not mint (see `parseBearerRaw`).
const TOKEN_RE = /^[A-Za-z0-9._~+/=-]{16,512}$/;
const BEARER_RE = /^Bearer[ \t]+([^\s]+)[ \t]*$/i;
/// Ceiling on a credential we only ever compare, never hash or store. Long
/// enough for any passphrase, short enough that a huge header cannot make us
/// allocate.
const RAW_CREDENTIAL_MAX = 4096;

export function sha256Hex(value) {
  return createHash("sha256").update(value, "utf8").digest("hex");
}

/// Extract the credential from an `Authorization` header value. Returns "" for a
/// missing, malformed, or non-Bearer header — callers treat that as "no token".
export function parseBearer(headerValue) {
  if (typeof headerValue !== "string") return "";
  const m = BEARER_RE.exec(headerValue.trim());
  if (!m) return "";
  const token = m[1];
  return TOKEN_RE.test(token) ? token : "";
}

/// Extract a bearer credential WITHOUT applying `TOKEN_RE`. The maintainer
/// credential is operator-chosen (a password manager happily emits `Xk9#mQ2$…`),
/// and filtering it through the charset of our own token format turned a valid
/// secret into a permanent, indistinguishable 401 at the moment revocation was
/// needed. Only the HTTP bearer grammar applies here; the value is compared in
/// constant time and never hashed, stored, or used as key material.
export function parseBearerRaw(headerValue) {
  if (typeof headerValue !== "string" || headerValue.length > RAW_CREDENTIAL_MAX) return "";
  const m = BEARER_RE.exec(headerValue.trim());
  return m ? m[1] : "";
}

export function mintTokenValue() {
  return TOKEN_PREFIX + randomBytes(32).toString("base64url");
}

export function hashToken(token) {
  return sha256Hex(token);
}

function newAccountId() {
  return "acct_" + randomBytes(16).toString("base64url");
}

/// Constant-time comparison of two hex digests of equal length.
export function secretsEqual(a, b) {
  if (typeof a !== "string" || typeof b !== "string") return false;
  const left = Buffer.from(a, "utf8");
  const right = Buffer.from(b, "utf8");
  if (left.length !== right.length) return false;
  return timingSafeEqual(left, right);
}

export function isVerifiedKind(kind) {
  return kind !== KIND_ANONYMOUS && typeof kind === "string" && kind.length > 0;
}

async function writeToken(store, account, now) {
  const token = mintTokenValue();
  const tokenHash = hashToken(token);
  await store.setJson(KEYS.token(tokenHash), {
    token_hash: tokenHash,
    account_id: account.account_id,
    kind: account.kind,
    created_at: now,
    revoked_at: "",
  });
  return token;
}

/// Mint a fresh anonymous account plus its first token.
export async function createAnonymousAccount(store, now) {
  const account = {
    account_id: newAccountId(),
    kind: KIND_ANONYMOUS,
    provider: "",
    subject_hash: "",
    created_at: now,
    revoked_at: "",
  };
  await store.setJson(KEYS.account(account.account_id), account);
  const token = await writeToken(store, account, now);
  return { account, token };
}

/// Resolve (or create) the account for a verified external identity, then mint a
/// token for it. Re-authenticating with the same identity MUST land on the same
/// `account_id` — otherwise one GitHub user could log in N times and manufacture
/// N "independent" corroborations.
///
/// The subject pointer is therefore claimed with an atomic create-if-absent, not
/// a read-then-write: N simultaneous first logins for one identity would all read
/// an absent pointer and all mint their own account, and each of those accounts
/// would be permanently live (`verifyToken` resolves token -> account and never
/// consults the pointer). Exactly one caller can win the claim; the losers adopt
/// the winner's account and delete the record they speculatively wrote.
///
/// If the pointer exists but its account record does not, this fails closed
/// (`conflict`) rather than minting a second account for the same identity:
/// answering "retry" costs one sign-in, silently splitting an identity costs the
/// meaning of Community-reported.
export async function upsertVerifiedAccount(store, { provider, subject, now }) {
  const subjectHash = sha256Hex(`${provider}:${subject}`);
  const pointerKey = KEYS.subject(provider, subjectHash);
  let account = await resolvePointer(store, pointerKey);

  if (!account) {
    // Written before the claim so the winner's record is already readable by the
    // time any loser follows the pointer. Losing means nothing references this id
    // — no pointer, no token — so deleting it is safe and leaves no orphan.
    const candidate = {
      account_id: newAccountId(),
      kind: provider,
      provider,
      subject_hash: subjectHash,
      created_at: now,
      revoked_at: "",
    };
    await store.setJson(KEYS.account(candidate.account_id), candidate);
    const won = await store.setJsonIfAbsent(pointerKey, { account_id: candidate.account_id });
    if (won) {
      account = candidate;
    } else {
      await store.del(KEYS.account(candidate.account_id));
      account = await resolvePointer(store, pointerKey);
      if (!account) return { account: null, token: "", revoked: false, conflict: true };
    }
  }
  if (account.revoked_at) return { account, token: "", revoked: true, conflict: false };

  const token = await writeToken(store, account, now);
  return { account, token, revoked: false, conflict: false };
}

async function resolvePointer(store, pointerKey) {
  const pointer = await store.getJson(pointerKey);
  if (!pointer || typeof pointer.account_id !== "string") return null;
  return await store.getJson(KEYS.account(pointer.account_id));
}

/// Verify a presented bearer token. Returns a `reason` on failure purely for the
/// response body; every failure maps to the same 401 so nothing is leaked about
/// which part was wrong.
export async function verifyToken(store, token) {
  if (!token || !TOKEN_RE.test(token)) return { ok: false, reason: "malformed token" };
  const tokenHash = hashToken(token);
  const record = await store.getJson(KEYS.token(tokenHash));
  if (!record || typeof record.token_hash !== "string") return { ok: false, reason: "unknown token" };
  // Defense in depth: the lookup already matched by hash, but comparing the
  // recomputed digest against the stored one in constant time means a store that
  // ever answered a prefix/fuzzy match could not be used as an oracle.
  if (!secretsEqual(tokenHash, record.token_hash)) return { ok: false, reason: "unknown token" };
  if (record.revoked_at) return { ok: false, reason: "token revoked" };

  const account = await store.getJson(KEYS.account(record.account_id));
  if (!account) return { ok: false, reason: "unknown account" };
  if (account.revoked_at) return { ok: false, reason: "account revoked" };

  return { ok: true, reason: "", account, tokenHash };
}

/// Self-service revocation of the presented credential only. The account and its
/// published results survive, so rotating a token is not destructive.
export async function revokeToken(store, token, now) {
  const verified = await verifyToken(store, token);
  if (!verified.ok) return false;
  const key = KEYS.token(verified.tokenHash);
  const record = await store.getJson(key);
  if (!record) return false;
  await store.setJson(key, { ...record, revoked_at: now });
  return true;
}

/// Maintainer abuse response: kill the account itself. Every token bound to it
/// stops working on the next request because `verifyToken` re-reads the account.
export async function revokeAccount(store, accountId, now) {
  const account = await store.getJson(KEYS.account(accountId));
  if (!account) return null;
  if (account.revoked_at) return account;
  const updated = { ...account, revoked_at: now };
  await store.setJson(KEYS.account(accountId), updated);
  return updated;
}

// ---- GitHub OAuth device flow -------------------------------------------
// Feature-flagged on GITHUB_CLIENT_ID. The device flow's token exchange takes
// only the client id (GitHub does not use a secret for it), so gating on the
// secret as well would lock out a correctly configured public OAuth app; we send
// the secret only when one is configured.

export function githubConfig(env) {
  const clientId = (env.GITHUB_CLIENT_ID || "").trim();
  const clientSecret = (env.GITHUB_CLIENT_SECRET || "").trim();
  const minAccountAgeDays = numberFromEnv(env.GITHUB_MIN_ACCOUNT_AGE_DAYS, 30);
  return { clientId, clientSecret, minAccountAgeDays, enabled: clientId.length > 0 };
}

function numberFromEnv(raw, fallback) {
  const n = Number.parseInt(String(raw ?? ""), 10);
  return Number.isFinite(n) && n >= 0 ? n : fallback;
}

async function postForm(url, params, fetchImpl) {
  const res = await fetchImpl(url, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded", Accept: "application/json" },
    body: new URLSearchParams(params).toString(),
    signal: AbortSignal.timeout(GITHUB_TIMEOUT_MS),
  });
  const body = await res.json().catch(() => ({}));
  return { status: res.status, body: body && typeof body === "object" ? body : {} };
}

/// Start a device authorization. Returns what the CLI needs to print.
export async function githubDeviceStart(env, fetchImpl = fetch) {
  const cfg = githubConfig(env);
  if (!cfg.enabled) return { ok: false, reason: "github sign-in is not configured on this registry" };
  const { status, body } = await postForm(GITHUB_DEVICE_CODE_URL, { client_id: cfg.clientId, scope: "" }, fetchImpl);
  if (status !== 200 || !body.device_code || !body.user_code) {
    return { ok: false, reason: "github device authorization could not be started" };
  }
  return {
    ok: true,
    device_code: String(body.device_code),
    user_code: String(body.user_code),
    verification_uri: String(body.verification_uri || "https://github.com/login/device"),
    interval: Number(body.interval) || 5,
    expires_in: Number(body.expires_in) || 900,
  };
}

/// Exchange an approved device code for the GitHub user's stable id. Returns a
/// `state` the route maps onto a status code; the GitHub access token never
/// leaves this function.
export async function githubExchange(env, deviceCode, fetchImpl = fetch) {
  const cfg = githubConfig(env);
  if (!cfg.enabled) return { state: "unconfigured", reason: "github sign-in is not configured on this registry" };

  const params = {
    client_id: cfg.clientId,
    device_code: deviceCode,
    grant_type: "urn:ietf:params:oauth:grant-type:device_code",
  };
  if (cfg.clientSecret) params.client_secret = cfg.clientSecret;

  const exchange = await postForm(GITHUB_TOKEN_URL, params, fetchImpl);
  const err = typeof exchange.body.error === "string" ? exchange.body.error : "";
  if (err === "authorization_pending") return { state: "pending", interval: Number(exchange.body.interval) || 5 };
  if (err === "slow_down") return { state: "slow_down", interval: Number(exchange.body.interval) || 10 };
  if (err === "expired_token") return { state: "expired", reason: "the device code expired; start sign-in again" };
  if (err === "access_denied") return { state: "denied", reason: "sign-in was cancelled" };
  const accessToken = typeof exchange.body.access_token === "string" ? exchange.body.access_token : "";
  if (!accessToken) return { state: "failed", reason: "github did not return an access token" };

  const userRes = await fetchImpl(GITHUB_USER_URL, {
    headers: {
      Authorization: `Bearer ${accessToken}`,
      Accept: "application/vnd.github+json",
      "User-Agent": "skillrank-registry",
    },
    signal: AbortSignal.timeout(GITHUB_TIMEOUT_MS),
  });
  if (!userRes.ok) return { state: "failed", reason: "could not read the github account" };
  const user = await userRes.json().catch(() => null);
  if (!user || (typeof user.id !== "number" && typeof user.id !== "string")) {
    return { state: "failed", reason: "could not read the github account" };
  }

  // Independence has to cost something (R5): a GitHub account created minutes ago
  // is as cheap as an anonymous token, so it does not get verified standing.
  const ageDays = accountAgeDays(user.created_at);
  if (cfg.minAccountAgeDays > 0 && ageDays !== null && ageDays < cfg.minAccountAgeDays) {
    return {
      state: "ineligible",
      reason: `github accounts must be at least ${cfg.minAccountAgeDays} days old to publish`,
    };
  }

  return { state: "ok", subject: String(user.id) };
}

function accountAgeDays(createdAt) {
  const ts = Date.parse(String(createdAt || ""));
  if (!Number.isFinite(ts)) return null;
  return (Date.now() - ts) / 86_400_000;
}
