package runner

import (
	"context"
	"os"
	"path/filepath"
	"testing"

	reg "github.com/buildbetter/skillrank/internal/registry"
)

// stubFixture prepares a temp workspace containing only a fixture file.
type stubFixture struct{ dirs []string }

func (s *stubFixture) Isolation() Isolation { return IsolationWorktree }
func (s *stubFixture) Prepare(_ context.Context, _ string) (string, func(), error) {
	dir, err := os.MkdirTemp("", "stub-fixture-")
	if err != nil {
		return "", func() {}, err
	}
	_ = os.WriteFile(filepath.Join(dir, "README.md"), []byte("fixture"), 0o644)
	s.dirs = append(s.dirs, dir)
	return dir, func() {}, nil
}

// stubAgent writes solution.txt when the skill is installed (treatment), and
// records whether any verifier content leaked into the workspace during the run.
type stubAgent struct {
	sawVerifierInWorkspace bool
}

func (a *stubAgent) AgentName() string        { return "claude_code" }
func (a *stubAgent) AgentVersionBand() string { return "2.1" }
func (a *stubAgent) RunTask(_ context.Context, spec RunSpec) (RunOutcome, error) {
	// Verifier isolation check: no verify.sh should exist in the workspace while
	// the agent is running.
	_ = filepath.Walk(spec.WorkingDir, func(path string, info os.FileInfo, err error) error {
		if err == nil && info != nil && !info.IsDir() && filepath.Base(path) == "verify.sh" {
			a.sawVerifierInWorkspace = true
		}
		return nil
	})
	tokens := int64(1000)
	if spec.SkillInstalled {
		_ = os.WriteFile(filepath.Join(spec.WorkingDir, "solution.txt"), []byte("done"), 0o644)
		tokens = 700 // treatment uses fewer tokens
	}
	return RunOutcome{InputTokens: tokens, OutputTokens: 100, Turns: 2, DurationMS: 10}, nil
}

func TestRunEvalArmsVerifierIsolationAndBundle(t *testing.T) {
	suite := reg.Suite{
		ID:      "launch/playwright",
		Version: "1",
		Fixture: reg.SuiteFixture{GitURL: "https://example/repo", Commit: "abc"},
		Tasks: []reg.SuiteTask{
			{ID: "task-a", Instruction: "do a", TimeoutSec: 60, EstTokens: 1000, EstCostUSD: 0.01},
			{ID: "task-b", Instruction: "do b", TimeoutSec: 60, EstTokens: 1000, EstCostUSD: 0.01},
		},
		ReferenceEnv: reg.ReferenceEnv{AgentVersionBand: "2.1", Models: []string{"sonnet"}},
	}
	skill := reg.ResolveResponse{Slug: "owner/skill", ContentHash: "sha256:aaa", InlineContent: "---\nname: skill\n---\nbody"}
	agent := &stubAgent{}
	fixtures := &stubFixture{}
	// Real verifier: passes only when the treatment arm produced solution.txt.
	verifier := &ScriptVerifier{Commands: map[string]string{
		"task-a": `test -f "$1/solution.txt"`,
		"task-b": `test -f "$1/solution.txt"`,
	}}

	cfg := Config{Trials: 3, Model: "sonnet"}
	result, err := RunEval(context.Background(), suite, skill, cfg, agent, fixtures, verifier)
	if err != nil {
		t.Fatalf("RunEval: %v", err)
	}

	// 2 tasks × 3 trials × 2 arms = 12 trial records.
	if len(result.Bundle.Trials) != 12 {
		t.Fatalf("expected 12 trial records, got %d", len(result.Bundle.Trials))
	}
	// Verifier isolation: the agent must never have seen verify.sh.
	if agent.sawVerifierInWorkspace {
		t.Fatal("verifier content leaked into the agent workspace (isolation broken)")
	}
	// Arm behavior: control fails (no solution), treatment passes → +100pp delta.
	for _, d := range result.Report.Deltas {
		if d.ControlPassRate != 0 {
			t.Errorf("task %s: expected control pass rate 0, got %f", d.TaskID, d.ControlPassRate)
		}
		if d.TreatmentPassRate != 1 {
			t.Errorf("task %s: expected treatment pass rate 1, got %f", d.TaskID, d.TreatmentPassRate)
		}
		if d.TokenDeltaPct >= 0 {
			t.Errorf("task %s: expected negative token delta (treatment cheaper), got %f", d.TaskID, d.TokenDeltaPct)
		}
	}
	// Low-N caveat at 3 trials.
	if !result.Report.LowNCaveat {
		t.Error("expected low-N caveat at 3 trials/arm")
	}
	// Worktree isolation → not conforming (Self-reported only).
	if result.Bundle.Conforming {
		t.Error("worktree-isolation runs must not be marked conforming")
	}
	// Environment cell recorded.
	if result.Bundle.EnvironmentCell.Isolation != string(IsolationWorktree) {
		t.Errorf("unexpected isolation in cell: %s", result.Bundle.EnvironmentCell.Isolation)
	}
	if result.Bundle.Harness.Name != HarnessName {
		t.Errorf("harness name not recorded: %s", result.Bundle.Harness.Name)
	}
}

func TestConfigHashDeterministicAndSensitive(t *testing.T) {
	suite := reg.Suite{ID: "s", Version: "1"}
	skill := reg.ResolveResponse{Slug: "sk", ContentHash: "h"}
	cell := reg.EnvironmentCell{Agent: "claude_code", AgentVersionBand: "2.1", Model: "sonnet", OS: "darwin", Isolation: "docker"}
	h1 := ComputeConfigHash(suite, skill, Config{Trials: 3, Model: "sonnet"}, cell)
	h2 := ComputeConfigHash(suite, skill, Config{Trials: 3, Model: "sonnet"}, cell)
	if h1 != h2 {
		t.Fatal("config hash must be deterministic")
	}
	// The recorded model lives in the environment cell (RunEval populates it from
	// cfg.Model); the hash must change when the cell's model changes.
	cellOpus := cell
	cellOpus.Model = "opus"
	h3 := ComputeConfigHash(suite, skill, Config{Trials: 3, Model: "opus"}, cellOpus)
	if h1 == h3 {
		t.Fatal("config hash must change with the model")
	}
	// Trials are also part of the config identity.
	h4 := ComputeConfigHash(suite, skill, Config{Trials: 5, Model: "sonnet"}, cell)
	if h1 == h4 {
		t.Fatal("config hash must change with the trial count")
	}
}

func TestEstimateCost(t *testing.T) {
	suite := reg.Suite{Tasks: []reg.SuiteTask{{EstTokens: 1000, EstCostUSD: 0.02}, {EstTokens: 2000, EstCostUSD: 0.04}}}
	tokens, cost := EstimateCost(suite, Config{Trials: 3})
	// (1000+2000) tokens * 3 trials * 2 arms = 18000
	if tokens != 18000 {
		t.Fatalf("expected 18000 tokens, got %d", tokens)
	}
	if cost < 0.35 || cost > 0.37 { // (0.02+0.04)*3*2 = 0.36
		t.Fatalf("expected ~0.36 cost, got %f", cost)
	}
}
