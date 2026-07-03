package runner

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"

	reg "github.com/buildbetter/skillrank/internal/registry"
)

// ComputeConfigHash canonicalizes the run parameters that must match for two
// bundles to be treated as the same configuration (dedup key on ingest). It is
// deterministic and independent of trial outcomes.
func ComputeConfigHash(suite reg.Suite, skill reg.ResolveResponse, cfg Config, cell reg.EnvironmentCell) string {
	canonical := fmt.Sprintf(
		"harness=%s/%s|suite=%s@%s|skill=%s@%s|trials=%d|agent=%s|band=%s|model=%s|os=%s|isolation=%s",
		HarnessName, HarnessVersion,
		suite.ID, suite.Version,
		skill.Slug, skill.ContentHash,
		cfg.Trials,
		cell.Agent, cell.AgentVersionBand, cell.Model, cell.OS, cell.Isolation,
	)
	sum := sha256.Sum256([]byte(canonical))
	return "sha256:" + hex.EncodeToString(sum[:])
}
