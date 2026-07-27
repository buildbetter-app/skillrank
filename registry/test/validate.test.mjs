import assert from "node:assert/strict";
import test from "node:test";

import { computeConfigHash, ingestConfig, isConforming, validateBundleShape } from "../lib/ingest.mjs";
import { SUITES, makeBundle } from "./helpers.mjs";

const CONFIG = ingestConfig({});
const NOW = Date.parse("2026-07-24T18:00:00Z");

function check(bundle, overrides = {}) {
  return validateBundleShape(bundle, { suites: SUITES, config: CONFIG, now: NOW, ...overrides });
}

function fresh(extra = {}) {
  return makeBundle({ createdAt: "2026-07-24T17:00:00Z", ...extra });
}

test("config_hash matches the Rust runner byte for byte", () => {
  // Independently computed from skillrank_core::runner::compute_config_hash.
  assert.equal(
    computeConfigHash({
      harness: { name: "skillrank-runner", version: "0.1.0" },
      suite: { id: "debug-basics", version: "1" },
      slug: "obra/systematic-debugging",
      contentHash: "sha256:0a51b2cb622ce3de76c292cffb3fdc803c34cd995b601737d648e03035db22f3",
      trialsPerArm: 3,
      cell: {
        agent: "claude_code",
        agent_version_band: "2.1",
        model: "claude-sonnet-5",
        os: "macos",
        isolation: "docker",
      },
    }),
    "sha256:267af9d1aa9d7d2fb74ec5040f5394a1a595ec6d964a343992bfb54d5f08bc04",
  );
});

test("isConforming mirrors the runner's rule", () => {
  const reference = { agent_version_band: "", models: ["claude-sonnet-5"] };
  const docker = { agent: "claude_code", agent_version_band: "2.1", model: "claude-sonnet-5", os: "macos", isolation: "docker" };
  assert.equal(isConforming(reference, docker), true);
  assert.equal(isConforming(reference, { ...docker, isolation: "worktree" }), false);
  assert.equal(isConforming(reference, { ...docker, model: "gpt-5" }), false);
  assert.equal(isConforming({ agent_version_band: "3.0", models: [] }, docker), false);
  assert.equal(isConforming({ agent_version_band: "2.1", models: [] }, docker), true);
});

test("a real bundle passes", () => {
  const result = check(fresh());
  assert.equal(result.ok, true, result.reason);
  assert.equal(result.value.trialsPerArm, 3);
  assert.equal(result.value.conforming, false, "worktree runs are never conforming");
  assert.deepEqual(result.value.totals, {
    control_pass: 1,
    control_total: 3,
    control_tokens: 3000,
    treatment_pass: 2,
    treatment_total: 3,
    treatment_tokens: 2700,
  });
});

test("a docker bundle on the reference model is conforming", () => {
  const result = check(fresh({ isolation: "docker" }));
  assert.equal(result.ok, true, result.reason);
  assert.equal(result.value.conforming, true);
});

test("codex-shaped bundles with every optional field omitted still pass", () => {
  const bundle = fresh({ model: "", agent: "codex", band: "" });
  for (const t of bundle.trials) {
    delete t.cost_usd;
    delete t.trajectory_digest;
  }
  assert.equal(check(bundle).ok, true);
});

const rejections = [
  ["non-object body", () => "nope", "body must be a JSON object"],
  ["null body", () => null, "body must be a JSON object"],
  ["future bundle version", () => ({ ...fresh(), bundle_version: 2 }), /bundle_version/],
  ["unknown suite", () => ({ ...fresh(), suite_id: "nope" }), /unknown suite/],
  ["wrong suite version", () => fresh({ suiteVersion: "2" }), /version 1, not 2/],
  ["malformed slug", () => fresh({ slug: "../../etc/passwd" }), /skill_slug is malformed/],
  ["malformed content hash", () => ({ ...fresh(), skill_content_hash: "nope" }), /skill_content_hash/],
  ["foreign harness", () => ({ ...fresh(), harness: { name: "my-runner", version: "1" } }), /harness/],
  ["bad isolation", () => fresh({ isolation: "none" }), /isolation/],
  ["missing created_at", () => ({ ...fresh(), created_at: undefined }), /created_at/],
  ["stale created_at", () => fresh({ createdAt: "2026-01-01T00:00:00Z" }), /older than 30 days/],
  ["future created_at", () => fresh({ createdAt: "2027-01-01T00:00:00Z" }), /in the future/],
  ["too many trials", () => fresh({ trialsPerArm: 200 }), /at most 25 trials per arm/],
  ["empty trials", () => ({ ...fresh(), trials: [] }), /non-empty/],
];

for (const [name, build, expected] of rejections) {
  test(`rejects: ${name}`, () => {
    const result = check(build());
    assert.equal(result.ok, false);
    if (expected instanceof RegExp) assert.match(result.reason, expected);
    else assert.equal(result.reason, expected);
  });
}

test("rejects an unbalanced arm split", () => {
  const bundle = fresh();
  bundle.trials[0].arm = "treatment";
  const result = check(bundle);
  assert.equal(result.ok, false);
  assert.match(result.reason, /unbalanced/);
});

test("rejects a trial count that is not a whole number of paired trials", () => {
  const bundle = fresh();
  bundle.trials.pop();
  const result = check(bundle);
  assert.equal(result.ok, false);
  assert.match(result.reason, /multiple of 2/);
});

test("rejects a task id the suite does not declare", () => {
  const bundle = fresh();
  bundle.trials[0].task_id = "some-other-task";
  assert.match(check(bundle).reason, /not in suite/);
});

test("rejects an unrecognized verdict", () => {
  const bundle = fresh();
  bundle.trials[0].verdict = "probably-passed";
  assert.match(check(bundle).reason, /verdict/);
});

test("rejects negative counters", () => {
  const bundle = fresh();
  bundle.trials[0].input_tokens = -5;
  assert.match(check(bundle).reason, /non-negative/);
});

test("a tampered config_hash is caught", () => {
  const bundle = fresh();
  bundle.config_hash = `sha256:${"a".repeat(64)}`;
  assert.match(check(bundle).reason, /config_hash does not describe this run/);
});

test("claiming a different environment cell invalidates config_hash", () => {
  // The attack this blocks: run on worktree, relabel as docker to look conforming.
  const bundle = fresh({ isolation: "worktree" });
  bundle.environment_cell.isolation = "docker";
  assert.match(check(bundle).reason, /config_hash does not describe this run/);
});

test("claiming a different trial count invalidates config_hash", () => {
  const bundle = fresh({ trialsPerArm: 3 });
  bundle.trials.push({ ...bundle.trials[0] }, { ...bundle.trials[3] });
  assert.match(check(bundle).reason, /config_hash does not describe this run/);
});
