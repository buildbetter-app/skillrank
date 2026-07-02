// Package skillregistry implements the client, lockfile, install, and eval-runner
// logic for the SkillRank community skill registry. It is consumed by the
// `bb skills` command family and the standalone `skillrank` binary from a single
// implementation.
//
// The wire types in this file are the source-of-truth contract that the
// rest-api `/v3/rest/skill-registry/*` controllers serve. Field names use the
// registry's snake_case JSON convention.
package registry

// TrustTier labels how much a published eval result has been vouched for. Tiers are
// never mixed in aggregates.
type TrustTier string

const (
	// TierOfficial: executed by BuildBetter on the reference environment.
	TierOfficial TrustTier = "official"
	// TierCommunityReported: aggregated from >=3 distinct non-author accounts in one
	// environment cell; not reproduced by BuildBetter.
	TierCommunityReported TrustTier = "community_reported"
	// TierSelfReported: everything else (author-linked, non-Docker, non-conforming,
	// sub-threshold, or cells without an official baseline).
	TierSelfReported TrustTier = "self_reported"
)

// ScanTier is the published static-scan verdict for a skill version.
type ScanTier string

const (
	ScanSafe    ScanTier = "safe"
	ScanLow     ScanTier = "low"
	ScanMedium  ScanTier = "medium"
	ScanHigh    ScanTier = "high"
	ScanFlagged ScanTier = "flagged"
	ScanPending ScanTier = "pending"
	ScanUnknown ScanTier = "unknown"
)

// SkillSummary is one row in a search result.
type SkillSummary struct {
	Slug          string    `json:"slug"`
	DisplayName   string    `json:"display_name"`
	Category      string    `json:"category,omitempty"`
	Stacks        []string  `json:"stacks,omitempty"`
	SourceType    string    `json:"source_type"` // "github" | "hosted_private"
	SourceURL     string    `json:"source_url,omitempty"`
	LatestVersion string    `json:"latest_version"` // content hash
	ScanTier      ScanTier  `json:"scan_tier"`
	SignalsScore  *float64  `json:"signals_score,omitempty"`
	RatingAverage *float64  `json:"rating_average,omitempty"`
	RatingCount   int       `json:"rating_count"`
	Summary       string    `json:"summary,omitempty"`
}

// SearchResponse is the paginated search payload.
type SearchResponse struct {
	Items      []SkillSummary `json:"items"`
	NextCursor string         `json:"next_cursor,omitempty"`
	Total      int            `json:"total"`
}

// SkillVersion describes a single content-hashed version.
type SkillVersion struct {
	ContentHash  string   `json:"content_hash"`
	PinnedCommit string   `json:"pinned_commit,omitempty"`
	ScanTier     ScanTier `json:"scan_tier"`
	SignalsScore *float64 `json:"signals_score,omitempty"`
	PublishedAt  string   `json:"published_at,omitempty"`
}

// EvalSummaryCell is one (tier, cell) rollup shown on a skill page.
type EvalSummaryCell struct {
	Tier             TrustTier `json:"tier"`
	Agent            string    `json:"agent"`
	AgentVersionBand string    `json:"agent_version_band,omitempty"`
	Model            string    `json:"model"`
	Suite            string    `json:"suite"`
	SuiteVersion     string    `json:"suite_version"`
	NAccounts        int       `json:"n_accounts"`
	NTrials          int       `json:"n_trials"`
	SuccessLiftPct   *float64  `json:"success_lift_pct,omitempty"`
	NetTokenDeltaPct *float64  `json:"net_token_delta_pct,omitempty"`
	Gated            bool      `json:"gated"`
}

// SkillDetail is the full skill page payload.
type SkillDetail struct {
	SkillSummary
	Versions   []SkillVersion    `json:"versions"`
	EvalCells  []EvalSummaryCell `json:"eval_cells"`
	ScanReport map[string]any    `json:"scan_report,omitempty"`
}

// ResolveResponse is what `install` needs to fetch and verify a skill.
type ResolveResponse struct {
	Slug          string   `json:"slug"`
	Version       string   `json:"version"` // content hash of the resolved version
	SourceType    string   `json:"source_type"`
	SourceURL     string   `json:"source_url,omitempty"`
	SourceSubpath string   `json:"source_subpath,omitempty"`
	PinnedCommit  string   `json:"pinned_commit,omitempty"`
	ContentHash   string   `json:"content_hash"`
	ScanTier      ScanTier `json:"scan_tier"`
	SignalsScore  *float64 `json:"signals_score,omitempty"`
	// InlineContent is the SKILL.md body when the registry serves it directly
	// (source-mode skills). When empty, the client fetches RawContentURL.
	InlineContent  string `json:"inline_content,omitempty"`
	RawContentURL  string `json:"raw_content_url,omitempty"`
	Tombstoned     bool   `json:"tombstoned"`
	TombstoneReason string `json:"tombstone_reason,omitempty"`
}

// SuiteTask is one deterministic task in an eval suite. Only the public
// contract fields are served here; verifier test bodies and oracle solutions
// are fetched post-run from an authenticated runner-only endpoint.
type SuiteTask struct {
	ID               string   `json:"id"`
	Instruction      string   `json:"instruction"`
	VerifierContract string   `json:"verifier_contract"`
	TimeoutSec       int      `json:"timeout_sec"`
	EstTokens        int      `json:"est_tokens"`
	EstCostUSD       float64  `json:"est_cost_usd"`
}

// SuiteFixture pins the codebase an eval runs against.
type SuiteFixture struct {
	GitURL string `json:"git_url"`
	Commit string `json:"commit"`
	Image  string `json:"image,omitempty"`
}

// ReferenceEnv is the pinned agent/model band a run must match to be eligible
// for Community-reported aggregation.
type ReferenceEnv struct {
	AgentVersionBand string   `json:"agent_version_band"`
	Models           []string `json:"models"`
}

// Suite is a full eval suite definition (public fields only).
type Suite struct {
	ID           string       `json:"id"`
	Version      string       `json:"version"`
	Fixture      SuiteFixture `json:"fixture"`
	Tasks        []SuiteTask  `json:"tasks"`
	ReferenceEnv ReferenceEnv `json:"reference_env"`
}

// TrialArm identifies the treatment/control condition of a single trial.
type TrialArm string

const (
	ArmControl   TrialArm = "control"
	ArmTreatment TrialArm = "treatment"
)

// TrialRecord captures one agent run against one task in one arm.
type TrialRecord struct {
	TaskID       string   `json:"task_id"`
	Arm          TrialArm `json:"arm"`
	Verdict      string   `json:"verdict"` // "pass" | "fail" | "agent_error" | "verifier_error"
	InputTokens  int64    `json:"input_tokens"`
	OutputTokens int64    `json:"output_tokens"`
	CacheRead    int64    `json:"cache_read_tokens"`
	CacheWrite   int64    `json:"cache_write_tokens"`
	CostUSD      *float64 `json:"cost_usd,omitempty"`
	DurationMS   int64    `json:"duration_ms"`
	Turns        int      `json:"turns"`
	// TrajectoryDigest is a SHA-256 of the (locally-retained) trajectory; the
	// trajectory content itself never leaves the machine.
	TrajectoryDigest string `json:"trajectory_digest,omitempty"`
}

// EnvironmentCell keys where a bundle's results are comparable.
type EnvironmentCell struct {
	Agent            string `json:"agent"`
	AgentVersionBand string `json:"agent_version_band"`
	Model            string `json:"model"`
	OS               string `json:"os"`
	Isolation        string `json:"isolation"` // "docker" | "worktree"
}

// HarnessInfo records which runner produced a bundle.
type HarnessInfo struct {
	Name    string `json:"name"`
	Version string `json:"version"`
}

// EvalBundle is the versioned result artifact written locally and (optionally)
// published. It matches eval-bundle.schema.json served by the registry.
type EvalBundle struct {
	BundleVersion   int             `json:"bundle_version"`
	SkillSlug       string          `json:"skill_slug"`
	SkillContentHash string         `json:"skill_content_hash"`
	SuiteID         string          `json:"suite_id"`
	SuiteVersion    string          `json:"suite_version"`
	Harness         HarnessInfo     `json:"harness"`
	EnvironmentCell EnvironmentCell `json:"environment_cell"`
	Trials          []TrialRecord   `json:"trials"`
	ConfigHash      string          `json:"config_hash"`
	CreatedAt       string          `json:"created_at,omitempty"`
}

// IngestResponse is returned when a bundle is submitted.
type IngestResponse struct {
	Accepted   bool      `json:"accepted"`
	ResultID   string    `json:"result_id,omitempty"`
	TierState  string    `json:"tier_state,omitempty"`
	Reason     string    `json:"reason,omitempty"`
	Conforming bool      `json:"conforming"`
}

// PublishResponse is returned from source-mode or private-upload publishing.
type PublishResponse struct {
	Slug       string `json:"slug"`
	Version    string `json:"version,omitempty"`
	State      string `json:"state"` // "pending_scan" | "listed" | ...
	Visibility string `json:"visibility"` // "public" | "private"
	Message    string `json:"message,omitempty"`
}
