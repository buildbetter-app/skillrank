import assert from "node:assert/strict";
import test from "node:test";

import {
  createAnonymousAccount,
  hashToken,
  isVerifiedKind,
  parseBearer,
  parseBearerRaw,
  revokeAccount,
  revokeToken,
  secretsEqual,
  upsertVerifiedAccount,
  verifyToken,
} from "../lib/auth.mjs";
import { KEYS, Store, memoryClient } from "../lib/store.mjs";

function newStore() {
  return new Store(memoryClient());
}

const NOW = "2026-07-24T18:00:00Z";

test("parseBearer accepts only well-formed bearer credentials", () => {
  assert.equal(parseBearer("Bearer srk_abcdefghijklmnop"), "srk_abcdefghijklmnop");
  assert.equal(parseBearer("bearer srk_abcdefghijklmnop"), "srk_abcdefghijklmnop");
  assert.equal(parseBearer("Bearer   srk_abcdefghijklmnop  "), "srk_abcdefghijklmnop");
  assert.equal(parseBearer(""), "");
  assert.equal(parseBearer(undefined), "");
  assert.equal(parseBearer("Basic abcdefghijklmnop"), "");
  assert.equal(parseBearer("Bearer short"), "", "too short to be one of ours");
  assert.equal(parseBearer("Bearer srk_abcdefghijklmnop\r\nX-Evil: 1"), "", "no header injection");
});

test("a minted token verifies, and only the hash is persisted", async () => {
  const store = newStore();
  const { account, token } = await createAnonymousAccount(store, NOW);

  assert.ok(token.startsWith("srk_"));
  assert.ok(token.length > 40);
  const verified = await verifyToken(store, token);
  assert.equal(verified.ok, true);
  assert.equal(verified.account.account_id, account.account_id);
  assert.equal(verified.account.kind, "anonymous");
  assert.equal(isVerifiedKind(verified.account.kind), false);

  const record = await store.getJson(KEYS.token(hashToken(token)));
  assert.equal(record.token_hash, hashToken(token));
  assert.equal(JSON.stringify(record).includes(token), false, "plaintext token must never be stored");
});

test("unknown and malformed tokens are rejected", async () => {
  const store = newStore();
  assert.equal((await verifyToken(store, "srk_thisisnotarealtokenvalue")).ok, false);
  assert.equal((await verifyToken(store, "")).ok, false);
  assert.equal((await verifyToken(store, "nope")).ok, false);
});

test("revoking a token blocks it immediately without touching the account", async () => {
  const store = newStore();
  const { account, token } = await createAnonymousAccount(store, NOW);
  assert.equal(await revokeToken(store, token, NOW), true);

  const verified = await verifyToken(store, token);
  assert.equal(verified.ok, false);
  assert.equal(verified.reason, "token revoked");
  assert.ok(await store.getJson(KEYS.account(account.account_id)));
});

test("revoking an account kills every token bound to it", async () => {
  const store = newStore();
  const first = await upsertVerifiedAccount(store, { provider: "github", subject: "12345", now: NOW });
  const second = await upsertVerifiedAccount(store, { provider: "github", subject: "12345", now: NOW });
  assert.equal((await verifyToken(store, first.token)).ok, true);
  assert.equal((await verifyToken(store, second.token)).ok, true);

  await revokeAccount(store, first.account.account_id, NOW);
  assert.equal((await verifyToken(store, first.token)).reason, "account revoked");
  assert.equal((await verifyToken(store, second.token)).reason, "account revoked");
});

test("re-authenticating with the same identity reuses the account", async () => {
  const store = newStore();
  const first = await upsertVerifiedAccount(store, { provider: "github", subject: "999", now: NOW });
  const second = await upsertVerifiedAccount(store, { provider: "github", subject: "999", now: NOW });
  const other = await upsertVerifiedAccount(store, { provider: "github", subject: "1000", now: NOW });

  // If logging in twice minted two accounts, one GitHub user could manufacture
  // N "independent" corroborations by re-running `skillrank login`.
  assert.equal(first.account.account_id, second.account.account_id);
  assert.notEqual(first.token, second.token);
  assert.notEqual(first.account.account_id, other.account.account_id);
  assert.equal(first.account.subject_hash.length, 64);
  assert.equal(JSON.stringify(first.account).includes("999"), false, "raw subject is not stored");
});

test("a revoked verified account cannot mint a new token by signing in again", async () => {
  const store = newStore();
  const first = await upsertVerifiedAccount(store, { provider: "github", subject: "42", now: NOW });
  await revokeAccount(store, first.account.account_id, NOW);

  const retry = await upsertVerifiedAccount(store, { provider: "github", subject: "42", now: NOW });
  assert.equal(retry.revoked, true);
  assert.equal(retry.token, "");
});

test("secretsEqual is length-safe and value-correct", () => {
  assert.equal(secretsEqual("abc", "abc"), true);
  assert.equal(secretsEqual("abc", "abd"), false);
  assert.equal(secretsEqual("abc", "abcd"), false);
  assert.equal(secretsEqual("abc", undefined), false);
});

test("concurrent first logins for one identity mint exactly one account", async () => {
  // The exploit: `verifyToken` resolves token -> account and never consults the
  // subject pointer, so every account minted here would be permanently live. A
  // read-then-write claim lets N simultaneous first logins all observe an absent
  // pointer and all mint their own id — turning one GitHub user into N
  // "independent" accounts, which is exactly the quantity Community-reported
  // counts. The claim therefore has to be an atomic create-if-absent.
  const store = newStore();
  const logins = await Promise.all(
    Array.from({ length: 8 }, () => upsertVerifiedAccount(store, { provider: "github", subject: "777", now: NOW })),
  );

  const ids = new Set(logins.map((r) => r.account && r.account.account_id).filter(Boolean));
  assert.equal(ids.size, 1, "one identity must resolve to one account id, however concurrent the logins");
  assert.equal(logins.filter((r) => r.conflict).length, 0, "and none of them should have to fail closed");

  // Every issued token must resolve to that one account, and the speculative
  // records the losers wrote must be gone rather than left live and publishable.
  const [accountId] = [...ids];
  for (const login of logins) {
    const verified = await verifyToken(store, login.token);
    assert.equal(verified.ok, true);
    assert.equal(verified.account.account_id, accountId);
  }
  const pointer = await store.getJson(KEYS.subject("github", logins[0].account.subject_hash));
  assert.equal(pointer.account_id, accountId);

  // A second identity is still a second account — the fix must not collapse
  // distinct users onto one id.
  const other = await upsertVerifiedAccount(store, { provider: "github", subject: "778", now: NOW });
  assert.notEqual(other.account.account_id, accountId);
});

test("concurrent logins across two identities keep them separate", async () => {
  const store = newStore();
  const calls = [];
  for (const subject of ["alpha", "beta"]) {
    for (let i = 0; i < 6; i += 1) {
      calls.push(upsertVerifiedAccount(store, { provider: "github", subject, now: NOW }));
    }
  }
  const settled = await Promise.all(calls);
  const ids = new Set(settled.map((r) => r.account && r.account.account_id).filter(Boolean));
  assert.equal(ids.size, 2, "two identities, two accounts — no more, no fewer");
});

test("parseBearerRaw preserves a credential we did not mint", () => {
  // `parseBearer` applies the charset of the tokens WE mint. Running the
  // operator-chosen maintainer credential through it reduced any password-manager
  // secret to "" and produced a permanent 401 that looked exactly like a wrong
  // secret — during the one incident where revocation is the lever.
  const passwordManagerSecret = "Xk9#mQ2$vL8@nP4!";
  assert.equal(parseBearer(`Bearer ${passwordManagerSecret}`), "", "documents why a second parser is needed");
  assert.equal(parseBearerRaw(`Bearer ${passwordManagerSecret}`), passwordManagerSecret);
  assert.equal(secretsEqual(parseBearerRaw(`Bearer ${passwordManagerSecret}`), passwordManagerSecret), true);

  // Only the HTTP bearer grammar applies, and the value is bounded: it is compared
  // in constant time and never hashed, stored, or used as key material.
  assert.equal(parseBearerRaw("bearer  s3cr3t!%^&*()"), "s3cr3t!%^&*()");
  assert.equal(parseBearerRaw("Basic s3cr3t"), "");
  assert.equal(parseBearerRaw("Bearer a\r\nX-Evil: 1"), "", "no header injection");
  assert.equal(parseBearerRaw(`Bearer ${"a".repeat(5000)}`), "", "a huge header cannot make us allocate");
  assert.equal(parseBearerRaw(undefined), "");
});
