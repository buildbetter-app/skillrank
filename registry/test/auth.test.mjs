import assert from "node:assert/strict";
import test from "node:test";

import {
  createAnonymousAccount,
  hashToken,
  isVerifiedKind,
  parseBearer,
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
