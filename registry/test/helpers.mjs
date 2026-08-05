// Shared fixtures for the registry tests. The bundle builder mirrors exactly what
// `skillrank eval --publish` puts on the wire, including the alphabetical key
// order serde produces and the optional fields it omits.

import { computeConfigHash } from "../lib/ingest.mjs";

export const SLUG = "obra/systematic-debugging";
export const CONTENT_HASH = "sha256:0a51b2cb622ce3de76c292cffb3fdc803c34cd995b601737d648e03035db22f3";

export const SUITES = [
  {
    id: "debug-basics",
    version: "1",
    tasks: [{ id: "merge-intervals", instruction: "fix it", timeout_sec: 300 }],
    reference_env: { agent_version_band: "", models: ["claude-sonnet-5"] },
  },
];

export const CATALOG = {
  hasSkill: (slug) => slug === SLUG,
  publishedHash: (slug) => (slug === SLUG ? CONTENT_HASH : ""),
};

const HARNESS = { name: "skillrank-runner", version: "0.1.0" };

function trial(arm, verdict, tokens) {
  return {
    arm,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    duration_ms: 1000,
    input_tokens: tokens,
    output_tokens: 0,
    task_id: "merge-intervals",
    turns: 3,
    verdict,
  };
}

/// Build a wire-shaped bundle whose `config_hash` is genuinely correct for its
/// contents, so tests exercise the real validation path rather than a stub.
export function makeBundle({
  trialsPerArm = 3,
  harnessVersion = HARNESS.version,
  isolation = "worktree",
  model = "claude-sonnet-5",
  agent = "claude_code",
  band = "2.1",
  createdAt = new Date().toISOString().replace(/\.\d+Z$/, "Z"),
  controlPass = 1,
  treatmentPass = 2,
  controlTokens = 1000,
  treatmentTokens = 900,
  contentHash = CONTENT_HASH,
  slug = SLUG,
  suiteId = "debug-basics",
  suiteVersion = "1",
} = {}) {
  const cell = { agent, agent_version_band: band, model, os: "macos", isolation };
  const harness = { ...HARNESS, version: harnessVersion };
  const trials = [];
  for (let i = 0; i < trialsPerArm; i += 1) {
    trials.push(trial("control", i < controlPass ? "pass" : "fail", controlTokens));
  }
  for (let i = 0; i < trialsPerArm; i += 1) {
    trials.push(trial("treatment", i < treatmentPass ? "pass" : "fail", treatmentTokens));
  }
  return {
    bundle_version: 1,
    config_hash: computeConfigHash({
      harness,
      suite: { id: suiteId, version: suiteVersion },
      slug,
      contentHash,
      trialsPerArm,
      cell,
    }),
    created_at: createdAt,
    environment_cell: cell,
    harness: { ...harness },
    skill_content_hash: contentHash,
    skill_slug: slug,
    suite_id: suiteId,
    suite_version: suiteVersion,
    trials,
  };
}

export function account(id, kind = "github") {
  return { account_id: id, kind, verified: kind !== "anonymous" };
}
