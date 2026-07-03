package runner

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

// CLIAgentRunner invokes the user's own `claude` or `codex` binary as a one-shot,
// on the user's own subscription/credentials. It mirrors the argv shape used by
// the ZeroShot desktop app's run_agent_oneshot.
type CLIAgentRunner struct {
	Provider string // "claude" | "codex"
	Binary   string // resolved binary path/name
	Version  string // captured agent version
}

// AgentName implements AgentRunner.
func (r *CLIAgentRunner) AgentName() string {
	if r.Provider == "claude" {
		return "claude_code"
	}
	return r.Provider
}

// AgentVersionBand implements AgentRunner. The band collapses patch versions so a
// weekly agent release does not fragment every environment cell (minor-version
// band, mirroring the registry reference pin).
func (r *CLIAgentRunner) AgentVersionBand() string {
	return VersionBand(r.Version)
}

// VersionBand reduces a semver-ish version string to "major.minor".
func VersionBand(version string) string {
	v := strings.TrimSpace(version)
	v = strings.TrimPrefix(v, "v")
	parts := strings.SplitN(v, ".", 3)
	if len(parts) >= 2 {
		return parts[0] + "." + parts[1]
	}
	return v
}

// RunTask installs the skill for the treatment arm, invokes the agent one-shot,
// and parses usage. The verifier is applied by the caller AFTER this returns, so
// the agent workspace never contains verifier content.
func (r *CLIAgentRunner) RunTask(ctx context.Context, spec RunSpec) (RunOutcome, error) {
	if spec.SkillInstalled {
		if err := installSkillIntoWorkspace(spec.WorkingDir, spec.SkillSlug, spec.SkillContent); err != nil {
			return RunOutcome{}, err
		}
	}
	prompt := buildPrompt(spec)

	timeout := time.Duration(spec.TimeoutSec) * time.Second
	if timeout <= 0 {
		timeout = 240 * time.Second
	}
	runCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()

	args := r.argv(spec)
	cmd := exec.CommandContext(runCtx, r.Binary, args...)
	cmd.Dir = spec.WorkingDir
	cmd.Stdin = strings.NewReader(prompt)

	start := time.Now()
	stdout, err := cmd.Output()
	elapsed := time.Since(start).Milliseconds()

	digest := sha256.Sum256(stdout)
	outcome := RunOutcome{
		DurationMS:       elapsed,
		TrajectoryDigest: "sha256:" + hex.EncodeToString(digest[:]),
	}
	if runCtx.Err() == context.DeadlineExceeded {
		outcome.AgentError = true
		return outcome, nil
	}
	if err != nil {
		// A non-zero exit is treated as an agent error for this trial rather than
		// failing the whole run — completed trials still produce a valid bundle.
		outcome.AgentError = true
		return outcome, nil
	}

	parsed, parseErr := ParseAgentUsage(r.Provider, stdout)
	if parseErr != nil {
		outcome.AgentError = true
		return outcome, nil
	}
	parsed.DurationMS = elapsed
	parsed.TrajectoryDigest = outcome.TrajectoryDigest
	return parsed, nil
}

func (r *CLIAgentRunner) argv(spec RunSpec) []string {
	if r.Provider == "claude" {
		args := []string{"-p", "--output-format", "json", "--dangerously-skip-permissions"}
		if spec.Model != "" {
			args = append(args, "--model", spec.Model)
		}
		return args
	}
	// codex
	args := []string{
		"exec", "--json", "--skip-git-repo-check", "--ignore-user-config",
		"--sandbox", "workspace-write",
	}
	if spec.Model != "" {
		args = append(args, "--model", spec.Model)
	}
	return args
}

func buildPrompt(spec RunSpec) string {
	if spec.SkillInstalled {
		// Forced mode: explicitly direct the agent to use the installed skill so we
		// measure content quality, not trigger/activation behavior.
		return fmt.Sprintf(
			"Use the skill at .claude/skills/%s/SKILL.md for this task.\n\n%s",
			spec.SkillSlug, spec.Instruction)
	}
	return spec.Instruction
}

func installSkillIntoWorkspace(workDir, slug, content string) error {
	if strings.TrimSpace(content) == "" {
		return fmt.Errorf("treatment arm requires skill content but none was provided")
	}
	dir := filepath.Join(workDir, ".claude", "skills", slug)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(dir, "SKILL.md"), []byte(content), 0o644)
}

// --- usage parsing (pure functions, unit-tested directly) ---

// ParseAgentUsage dispatches to the provider-specific parser.
func ParseAgentUsage(provider string, stdout []byte) (RunOutcome, error) {
	if provider == "claude" {
		return parseClaudeUsage(stdout)
	}
	return parseCodexUsage(stdout)
}

type claudeResult struct {
	TotalCostUSD *float64 `json:"total_cost_usd"`
	NumTurns     int      `json:"num_turns"`
	DurationMS   int64    `json:"duration_ms"`
	IsError      bool     `json:"is_error"`
	Usage        struct {
		InputTokens              int64 `json:"input_tokens"`
		OutputTokens             int64 `json:"output_tokens"`
		CacheReadInputTokens     int64 `json:"cache_read_input_tokens"`
		CacheCreationInputTokens int64 `json:"cache_creation_input_tokens"`
	} `json:"usage"`
}

func parseClaudeUsage(stdout []byte) (RunOutcome, error) {
	var res claudeResult
	if err := json.Unmarshal(stdout, &res); err != nil {
		// claude -p --output-format json emits a single result object; if a stream
		// slipped through, take the last JSON object on the last non-empty line.
		if obj, ok := lastJSONObject(stdout); ok {
			if err2 := json.Unmarshal(obj, &res); err2 != nil {
				return RunOutcome{}, fmt.Errorf("parse claude usage: %w", err)
			}
		} else {
			return RunOutcome{}, fmt.Errorf("parse claude usage: %w", err)
		}
	}
	return RunOutcome{
		InputTokens:  res.Usage.InputTokens,
		OutputTokens: res.Usage.OutputTokens,
		CacheRead:    res.Usage.CacheReadInputTokens,
		CacheWrite:   res.Usage.CacheCreationInputTokens,
		CostUSD:      res.TotalCostUSD,
		DurationMS:   res.DurationMS,
		Turns:        res.NumTurns,
		AgentError:   res.IsError,
	}, nil
}

type codexTurnCompleted struct {
	Type  string `json:"type"`
	Usage struct {
		InputTokens         int64 `json:"input_tokens"`
		CachedInputTokens   int64 `json:"cached_input_tokens"`
		OutputTokens        int64 `json:"output_tokens"`
		ReasoningOutputTokens int64 `json:"reasoning_output_tokens"`
	} `json:"usage"`
}

func parseCodexUsage(stdout []byte) (RunOutcome, error) {
	// codex exec --json emits JSONL; accumulate turn.completed usage and count
	// turns.
	lines := strings.Split(string(stdout), "\n")
	var out RunOutcome
	found := false
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" || !strings.HasPrefix(line, "{") {
			continue
		}
		var evt codexTurnCompleted
		if err := json.Unmarshal([]byte(line), &evt); err != nil {
			continue
		}
		if evt.Type == "turn.completed" {
			found = true
			out.InputTokens += evt.Usage.InputTokens
			out.OutputTokens += evt.Usage.OutputTokens + evt.Usage.ReasoningOutputTokens
			out.CacheRead += evt.Usage.CachedInputTokens
			out.Turns++
		}
	}
	if !found {
		return RunOutcome{}, fmt.Errorf("parse codex usage: no turn.completed events")
	}
	return out, nil
}

func lastJSONObject(data []byte) ([]byte, bool) {
	lines := strings.Split(string(data), "\n")
	for i := len(lines) - 1; i >= 0; i-- {
		line := strings.TrimSpace(lines[i])
		if strings.HasPrefix(line, "{") && strings.HasSuffix(line, "}") {
			return []byte(line), true
		}
	}
	return nil, false
}
