// Package runner is the single SkillRank eval harness. Official baselines,
// audit re-runs, and community runs all execute this code so their numbers are
// comparable. It runs forced-mode paired trials (control = no skill, treatment =
// skill installed) against a pinned fixture, with the verifier applied only after
// the agent process exits (verifier isolation), and produces an EvalBundle.
//
// The orchestrator depends on interfaces (AgentRunner, FixtureProvider, Verifier)
// so it is fully unit-testable with stubs; real implementations live in
// agent.go and fixture.go.
package runner

import (
	"context"
	"fmt"
	"runtime"
	"sort"

	reg "github.com/buildbetter/skillrank/internal/registry"
)

// HarnessName / HarnessVersion identify this runner in every bundle. They are
// part of the environment cell so official-vs-community results stay comparable.
const HarnessName = "skillrank-runner"

// HarnessVersion is the harness contract version, bumped on behavior changes.
const HarnessVersion = "0.1.0"

// Isolation describes how the fixture workspace was prepared.
type Isolation string

const (
	// IsolationDocker: fixture + verifier isolated in container layers.
	IsolationDocker Isolation = "docker"
	// IsolationWorktree: pinned-commit clone in a temp dir (no container). Results
	// from this mode are Self-reported only (cannot guarantee verifier isolation as
	// strongly as containers).
	IsolationWorktree Isolation = "worktree"
)

// RunSpec is one agent invocation request.
type RunSpec struct {
	WorkingDir     string
	Instruction    string
	Model          string
	SkillInstalled bool   // treatment arm installs the skill into the workspace surface
	SkillContent   string // SKILL.md content for the treatment arm
	SkillSlug      string
	TimeoutSec     int
}

// RunOutcome is the measured result of one agent invocation.
type RunOutcome struct {
	InputTokens      int64
	OutputTokens     int64
	CacheRead        int64
	CacheWrite       int64
	CostUSD          *float64
	DurationMS       int64
	Turns            int
	TrajectoryDigest string
	AgentError       bool
}

// Verdict is the deterministic scoring of one task after the agent run.
type Verdict struct {
	Pass          bool
	VerifierError bool
}

// AgentRunner invokes the user's coding agent for one task in one workspace.
type AgentRunner interface {
	// AgentName is the provider tag ("claude" | "codex").
	AgentName() string
	// AgentVersionBand is the reference-comparable version band.
	AgentVersionBand() string
	RunTask(ctx context.Context, spec RunSpec) (RunOutcome, error)
}

// FixtureProvider prepares an isolated workspace for one task arm and reports the
// isolation mode used.
type FixtureProvider interface {
	Prepare(ctx context.Context, taskID string) (workingDir string, cleanup func(), err error)
	Isolation() Isolation
}

// Verifier applies the (isolated, post-run) verifier to a completed workspace.
type Verifier interface {
	Verify(ctx context.Context, workingDir string, taskID string) (Verdict, error)
}

// Config parameterizes an eval run.
type Config struct {
	Trials int    // trials per arm per task
	Model  string // model id used and recorded
}

// Result carries the produced bundle plus a human-facing local report.
type Result struct {
	Bundle EvalBundleWithMeta
	Report Report
}

// EvalBundleWithMeta is the wire bundle plus non-published local metadata.
type EvalBundleWithMeta struct {
	reg.EvalBundle
	Conforming bool // whether the environment matched the suite reference env
}

// TaskDelta is the paired per-task summary printed locally.
type TaskDelta struct {
	TaskID            string  `json:"task_id"`
	ControlPassRate   float64 `json:"control_pass_rate"`
	TreatmentPassRate float64 `json:"treatment_pass_rate"`
	PassRateDelta     float64 `json:"pass_rate_delta"`
	ControlAvgTokens  float64 `json:"control_avg_tokens"`
	TreatmentAvgTokens float64 `json:"treatment_avg_tokens"`
	TokenDeltaPct     float64 `json:"token_delta_pct"`
}

// Report is the local paired analysis. It carries an explicit low-N caveat.
type Report struct {
	Deltas       []TaskDelta `json:"deltas"`
	TrialsPerArm int         `json:"trials_per_arm"`
	LowNCaveat   bool        `json:"low_n_caveat"`
	Isolation    Isolation   `json:"isolation"`
}

// EstimateCost returns the estimated token and USD range for a suite run.
func EstimateCost(suite reg.Suite, cfg Config) (tokens int, costUSD float64) {
	trials := cfg.Trials
	if trials <= 0 {
		trials = 3
	}
	for _, task := range suite.Tasks {
		// control + treatment arms.
		tokens += task.EstTokens * trials * 2
		costUSD += task.EstCostUSD * float64(trials) * 2
	}
	return tokens, costUSD
}

// RunEval executes forced-mode paired trials and builds a bundle + local report.
func RunEval(
	ctx context.Context,
	suite reg.Suite,
	skill reg.ResolveResponse,
	cfg Config,
	agent AgentRunner,
	fixtures FixtureProvider,
	verifier Verifier,
) (Result, error) {
	trials := cfg.Trials
	if trials <= 0 {
		trials = 3
	}
	if len(suite.Tasks) == 0 {
		return Result{}, fmt.Errorf("suite %s has no tasks", suite.ID)
	}

	var records []reg.TrialRecord
	// accumulators for the local report
	type acc struct {
		ctrlPass, ctrlTotal    int
		treatPass, treatTotal  int
		ctrlTokens, treatTokens float64
	}
	byTask := map[string]*acc{}
	order := []string{}

	for _, task := range suite.Tasks {
		byTask[task.ID] = &acc{}
		order = append(order, task.ID)
		for _, arm := range []reg.TrialArm{reg.ArmControl, reg.ArmTreatment} {
			for i := 0; i < trials; i++ {
				rec, err := runOneTrial(ctx, task, arm, skill, cfg, agent, fixtures, verifier)
				if err != nil {
					return Result{}, fmt.Errorf("task %s arm %s trial %d: %w", task.ID, arm, i+1, err)
				}
				records = append(records, rec)
				a := byTask[task.ID]
				tokens := float64(rec.InputTokens + rec.OutputTokens)
				if arm == reg.ArmControl {
					a.ctrlTotal++
					a.ctrlTokens += tokens
					if rec.Verdict == "pass" {
						a.ctrlPass++
					}
				} else {
					a.treatTotal++
					a.treatTokens += tokens
					if rec.Verdict == "pass" {
						a.treatPass++
					}
				}
			}
		}
	}

	cell := reg.EnvironmentCell{
		Agent:            agent.AgentName(),
		AgentVersionBand: agent.AgentVersionBand(),
		Model:            cfg.Model,
		OS:               runtime.GOOS,
		Isolation:        string(fixtures.Isolation()),
	}
	bundle := reg.EvalBundle{
		BundleVersion:    1,
		SkillSlug:        skill.Slug,
		SkillContentHash: skill.ContentHash,
		SuiteID:          suite.ID,
		SuiteVersion:     suite.Version,
		Harness:          reg.HarnessInfo{Name: HarnessName, Version: HarnessVersion},
		EnvironmentCell:  cell,
		Trials:           records,
		ConfigHash:       ComputeConfigHash(suite, skill, cfg, cell),
	}

	report := Report{TrialsPerArm: trials, LowNCaveat: trials < 5, Isolation: fixtures.Isolation()}
	for _, taskID := range order {
		a := byTask[taskID]
		d := TaskDelta{TaskID: taskID}
		if a.ctrlTotal > 0 {
			d.ControlPassRate = float64(a.ctrlPass) / float64(a.ctrlTotal)
			d.ControlAvgTokens = a.ctrlTokens / float64(a.ctrlTotal)
		}
		if a.treatTotal > 0 {
			d.TreatmentPassRate = float64(a.treatPass) / float64(a.treatTotal)
			d.TreatmentAvgTokens = a.treatTokens / float64(a.treatTotal)
		}
		d.PassRateDelta = d.TreatmentPassRate - d.ControlPassRate
		if d.ControlAvgTokens > 0 {
			d.TokenDeltaPct = (d.TreatmentAvgTokens - d.ControlAvgTokens) / d.ControlAvgTokens * 100
		}
		report.Deltas = append(report.Deltas, d)
	}
	sort.SliceStable(report.Deltas, func(i, j int) bool { return report.Deltas[i].TaskID < report.Deltas[j].TaskID })

	conforming := isConforming(suite.ReferenceEnv, cell)
	return Result{
		Bundle: EvalBundleWithMeta{EvalBundle: bundle, Conforming: conforming},
		Report: report,
	}, nil
}

func runOneTrial(
	ctx context.Context,
	task reg.SuiteTask,
	arm reg.TrialArm,
	skill reg.ResolveResponse,
	cfg Config,
	agent AgentRunner,
	fixtures FixtureProvider,
	verifier Verifier,
) (reg.TrialRecord, error) {
	workDir, cleanup, err := fixtures.Prepare(ctx, task.ID)
	if err != nil {
		return reg.TrialRecord{}, err
	}
	defer cleanup()

	spec := RunSpec{
		WorkingDir:     workDir,
		Instruction:    task.Instruction,
		Model:          cfg.Model,
		SkillInstalled: arm == reg.ArmTreatment,
		SkillContent:   skill.InlineContent,
		SkillSlug:      skill.Slug,
		TimeoutSec:     task.TimeoutSec,
	}
	outcome, err := agent.RunTask(ctx, spec)
	if err != nil {
		return reg.TrialRecord{}, err
	}

	rec := reg.TrialRecord{
		TaskID:           task.ID,
		Arm:              arm,
		InputTokens:      outcome.InputTokens,
		OutputTokens:     outcome.OutputTokens,
		CacheRead:        outcome.CacheRead,
		CacheWrite:       outcome.CacheWrite,
		CostUSD:          outcome.CostUSD,
		DurationMS:       outcome.DurationMS,
		Turns:            outcome.Turns,
		TrajectoryDigest: outcome.TrajectoryDigest,
	}
	if outcome.AgentError {
		rec.Verdict = "agent_error"
		return rec, nil
	}

	// Verifier isolation: only now, after the agent process has exited, do we
	// apply the verifier. Implementations must not expose verifier content to the
	// agent's workspace during RunTask.
	verdict, err := verifier.Verify(ctx, workDir, task.ID)
	if err != nil {
		rec.Verdict = "verifier_error"
		return rec, nil
	}
	if verdict.VerifierError {
		rec.Verdict = "verifier_error"
	} else if verdict.Pass {
		rec.Verdict = "pass"
	} else {
		rec.Verdict = "fail"
	}
	return rec, nil
}

func isConforming(ref reg.ReferenceEnv, cell reg.EnvironmentCell) bool {
	if cell.Isolation != string(IsolationDocker) {
		return false // non-Docker runs are Self-reported only
	}
	if ref.AgentVersionBand != "" && ref.AgentVersionBand != cell.AgentVersionBand {
		return false
	}
	if len(ref.Models) > 0 {
		ok := false
		for _, m := range ref.Models {
			if m == cell.Model {
				ok = true
				break
			}
		}
		if !ok {
			return false
		}
	}
	return true
}
