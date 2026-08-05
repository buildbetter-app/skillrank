// Persistence for the registry's write side (accounts, tokens, eval results).
//
// This module deliberately has NO third-party imports (node builtins only). It
// talks to an injected
// client that implements the handful of Redis commands we use, so the deployed
// function passes the real Upstash client while tests pass `memoryClient()`.
// That keeps every unit test runnable with zero `node_modules`, and it means
// there is no module-level singleton a test would have to reset.
//
// NOTE: this file lives in `registry/lib`, not `registry/api`. Vercel turns every
// source file under `api/` into its own public endpoint; helper modules must sit
// outside it. Vercel's file tracer follows the static import from
// `api/registry.mjs`, so these files still ship with the function.

import { createHash, randomBytes } from "node:crypto";

/// Key layout. Everything the write side owns is namespaced so it can never
/// collide with the pre-existing `installs` / `emails` keys.
export const KEYS = {
  // --- accounts & tokens ---
  token: (tokenHash) => `auth:token:${tokenHash}`,
  account: (accountId) => `auth:account:${accountId}`,
  subject: (provider, subjectHash) => `auth:subject:${provider}:${subjectHash}`,
  accountCells: (accountId) => `auth:account:${accountId}:cells`,

  // --- eval results ---
  // There is no separate idempotency-pointer key: `result_id` is a pure function
  // of the spec's R4 tuple, so the result record itself is the dedupe record and
  // cannot drift out of sync with a pointer.
  result: (resultId) => `eval:result:${resultId}`,
  cell: (cellId) => `eval:cell:${cellId}`,
  cellSummaries: (cellId) => `eval:cell:${cellId}:summaries`,
  skillCells: (slug) => `eval:skill:${slug}:cells`,
  skillHashes: (slug) => `eval:skill:${slug}:hashes`,

  // --- abuse control (fixed-window counters, always TTL'd) ---
  rate: (bucket) => `rl:${bucket}`,
};

/// Upstash deserializes JSON payloads on read; the in-memory client does not.
/// Accept either so the same code path works against both.
function decode(value) {
  if (value === null || value === undefined) return null;
  if (typeof value === "string") {
    try {
      return JSON.parse(value);
    } catch {
      return null;
    }
  }
  return value;
}

/// A typed facade over the raw client. All values are stored as JSON strings.
export class Store {
  constructor(client) {
    this.client = client;
  }

  async getJson(key) {
    return decode(await this.client.get(key));
  }

  async setJson(key, value, ttlSeconds = 0) {
    const opts = ttlSeconds > 0 ? { ex: ttlSeconds } : undefined;
    await this.client.set(key, JSON.stringify(value), opts);
  }

  /// Atomic create-if-absent (`SET ... NX`). Returns true only for the caller
  /// that created the key.
  ///
  /// This is the only safe way to claim a singleton pointer: a `getJson` +
  /// `setJson` pair looks equivalent but is two round trips, and N concurrent
  /// serverless invocations all read "absent" before any of them writes. That is
  /// exactly how one GitHub identity would end up with N account ids, i.e. N
  /// "independent" corroborations.
  async setJsonIfAbsent(key, value, ttlSeconds = 0) {
    const opts = { nx: true };
    if (ttlSeconds > 0) opts.ex = ttlSeconds;
    const res = await this.client.set(key, JSON.stringify(value), opts);
    // Upstash answers "OK" on a win and null when NX blocked the write.
    return res !== null && res !== undefined && res !== false;
  }

  async del(key) {
    await this.client.del(key);
  }

  /// Refresh a key's lifetime. Every `eval:*` key is written with a retention
  /// TTL and re-armed on each touch, so an abandoned cell eventually expires
  /// instead of pinning storage forever.
  async expire(key, ttlSeconds) {
    if (!(ttlSeconds > 0)) return;
    await this.client.expire(key, ttlSeconds);
  }

  async sadd(key, member) {
    await this.client.sadd(key, member);
  }

  async srem(key, member) {
    await this.client.srem(key, member);
  }

  async smembers(key) {
    const res = await this.client.smembers(key);
    return Array.isArray(res) ? res.map(String) : [];
  }

  async mgetJson(keys) {
    if (keys.length === 0) return [];
    const res = await this.client.mget(...keys);
    return (Array.isArray(res) ? res : []).map(decode);
  }

  async hsetJson(key, field, value) {
    await this.client.hset(key, { [field]: JSON.stringify(value) });
  }

  /// Remove one field. Used to evict a retained summary, which is what keeps a
  /// saturated cell admitting new corroboration instead of freezing.
  async hdel(key, field) {
    await this.client.hdel(key, field);
  }

  /// Returns `[{ field, value }]` rather than a merged object so callers can
  /// write an entry back without having to strip the field name out of it.
  async hgetallJson(key) {
    const raw = await this.client.hgetall(key);
    const out = [];
    for (const [field, value] of Object.entries(raw || {})) {
      const parsed = decode(value);
      if (parsed && typeof parsed === "object") out.push({ field, value: parsed });
    }
    return out;
  }

  async hlen(key) {
    const n = await this.client.hlen(key);
    return typeof n === "number" ? n : 0;
  }

  /// Fixed-window counter. Returns `retryAfter` in whole seconds when denied so
  /// the caller can set a `Retry-After` header the Rust client renders.
  async hit(bucket, limit, windowSeconds) {
    const key = KEYS.rate(bucket);
    const count = await this.client.incr(key);
    if (count === 1) await this.client.expire(key, windowSeconds);
    if (count <= limit) return { allowed: true, count, retryAfter: 0 };
    let ttl = await this.client.ttl(key);
    if (typeof ttl !== "number" || ttl < 0) ttl = windowSeconds;
    return { allowed: false, count, retryAfter: Math.max(1, Math.ceil(ttl)) };
  }
}

export function createStore(client) {
  return client ? new Store(client) : null;
}

function intEnv(raw, fallback) {
  const n = Number.parseInt(String(raw ?? ""), 10);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

/// Abuse-control budgets (spec R6). Read per request so a Vercel env change takes
/// effect without a redeploy. The GitHub poll budget is deliberately large: the
/// device flow polls every ~5s for up to 15 minutes.
export function rateLimits(env = {}) {
  return {
    // Anonymous tokens are cheap on purpose (they can never corroborate), so this
    // budget only has to stop bulk spam — not squeeze a shared office NAT where
    // several ZeroShot installs provision on the same day.
    anonTokensPerIpPerDay: intEnv(env.AUTH_ANON_TOKENS_PER_IP_PER_DAY, 20),
    deviceStartsPerIpPerHour: intEnv(env.AUTH_DEVICE_STARTS_PER_IP_PER_HOUR, 20),
    githubPollsPerIpPerHour: intEnv(env.AUTH_GITHUB_POLLS_PER_IP_PER_HOUR, 240),
    whoamiPerIpPerHour: intEnv(env.AUTH_WHOAMI_PER_IP_PER_HOUR, 120),
    publishesPerIpPerHour: intEnv(env.EVAL_WRITES_PER_IP_PER_HOUR, 60),
    publishesPerAccountPerHour: intEnv(env.EVAL_WRITES_PER_ACCOUNT_PER_HOUR, 30),
    publishesPerAccountPerDay: intEnv(env.EVAL_WRITES_PER_ACCOUNT_PER_DAY, 200),
  };
}

// ---- IP bucketing for the limiters ---------------------------------------
// These live here, next to `KEYS.rate` and `rateLimits`, rather than in the
// route file, because the security property is a property of the KEY NAME we
// persist — so it has to be assertable without booting an HTTP server.

/// Only reachable with no datastore configured, and in that case every write
/// route answers 503 before a limiter runs, so no digest is ever persisted.
/// Generated once per process so counters stay stable within an invocation.
const FALLBACK_IP_SALT = randomBytes(32).toString("base64url");

/// Salt for the rate-limit IP digest. It has to be SECRET, not merely present:
/// the digest IS the persisted `rl:` key name, and SHA-256 over a publicly known
/// constant salt is a 2^32 preimage search for IPv4 — so anyone who can read the
/// keyspace recovers every recent caller's address. A hardcoded default salt
/// (this used to fall back to the literal `"skillrank"`) therefore provides none
/// of the "no IPs, no PII" property it appears to.
///
/// Set `RATE_LIMIT_IP_SALT` to pin and rotate it. Otherwise it is derived from
/// the Upstash credential: already a deployment secret, already required for any
/// deployment that persists these keys at all, and stable across invocations so
/// the counters keep working.
export function ipSalt(env = {}) {
  const configured = String(env.RATE_LIMIT_IP_SALT || "").trim();
  if (configured) return configured;
  const derived = env.KV_REST_API_TOKEN || env.UPSTASH_REDIS_REST_TOKEN || "";
  return derived ? sha256Hex(`skillrank-ip-salt:${derived}`) : FALLBACK_IP_SALT;
}

/// Rate-limit bucket for the caller's IP. The address itself is never stored —
/// only a secret-salted digest, under the counter's short TTL.
export function clientIpBucket(req, env = {}) {
  const headers = (req && req.headers) || {};
  const forwarded = headers["x-vercel-forwarded-for"] || headers["x-forwarded-for"] || "";
  const raw = String(forwarded).split(",")[0].trim() || (req && req.socket && req.socket.remoteAddress) || "";
  return sha256Hex(`${ipSalt(env)}:${raw}`).slice(0, 32);
}

function sha256Hex(value) {
  return createHash("sha256").update(value, "utf8").digest("hex");
}

/// In-memory stand-in for the Upstash client, used by tests. Implements exactly
/// the command surface `Store` uses, including lazy TTL expiry.
export function memoryClient({ now = () => Date.now() } = {}) {
  const data = new Map();

  function live(key) {
    const entry = data.get(key);
    if (!entry) return null;
    if (entry.expiresAt && entry.expiresAt <= now()) {
      data.delete(key);
      return null;
    }
    return entry;
  }

  function upsert(key, make) {
    const entry = live(key);
    if (entry) return entry;
    const created = { value: make(), expiresAt: 0 };
    data.set(key, created);
    return created;
  }

  return {
    async get(key) {
      const entry = live(key);
      return entry ? entry.value : null;
    },
    async set(key, value, opts = {}) {
      const existing = live(key);
      if (opts.nx && existing) return null;
      const expiresAt = opts.ex ? now() + opts.ex * 1000 : 0;
      data.set(key, { value, expiresAt });
      return "OK";
    },
    async del(key) {
      data.delete(key);
      return 1;
    },
    async mget(...keys) {
      return keys.map((k) => {
        const entry = live(k);
        return entry ? entry.value : null;
      });
    },
    async sadd(key, member) {
      upsert(key, () => new Set()).value.add(String(member));
      return 1;
    },
    async srem(key, member) {
      const entry = live(key);
      if (entry) entry.value.delete(String(member));
      return 1;
    },
    async smembers(key) {
      const entry = live(key);
      return entry ? [...entry.value] : [];
    },
    async hset(key, obj) {
      const entry = upsert(key, () => new Map());
      for (const [field, value] of Object.entries(obj)) entry.value.set(field, value);
      return 1;
    },
    async hdel(key, field) {
      const entry = live(key);
      if (!entry) return 0;
      const removed = entry.value.delete(field) ? 1 : 0;
      if (entry.value.size === 0) data.delete(key);
      return removed;
    },
    async hgetall(key) {
      const entry = live(key);
      return entry ? Object.fromEntries(entry.value) : null;
    },
    async hlen(key) {
      const entry = live(key);
      return entry ? entry.value.size : 0;
    },
    async incr(key) {
      const entry = upsert(key, () => 0);
      entry.value = Number(entry.value) + 1;
      return entry.value;
    },
    async expire(key, seconds) {
      const entry = live(key);
      if (!entry) return 0;
      entry.expiresAt = now() + seconds * 1000;
      return 1;
    },
    async ttl(key) {
      const entry = live(key);
      if (!entry) return -2;
      if (!entry.expiresAt) return -1;
      return Math.ceil((entry.expiresAt - now()) / 1000);
    },
  };
}
