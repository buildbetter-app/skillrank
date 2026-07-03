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
}
