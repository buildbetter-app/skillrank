package skillscmd

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/buildbetter/skillrank/internal/command"
	reg "github.com/buildbetter/skillrank/internal/registry"
	"github.com/buildbetter/skillrank/internal/registry/runner"
)

func runEval(args []string, ctx command.Context) int {
	flags := command.ParseFlags(args)
	if len(flags.Positionals) == 0 {
		fmt.Fprintln(ctx.Stderr, "usage: eval <ref> --suite <id> [--trials N] [--agent claude|codex] [--model M] [--publish]")
		return 2
	}
	ref := flags.Positionals[0]
	suiteID := flags.Values["suite"]
	if strings.TrimSpace(suiteID) == "" {
		fmt.Fprintln(ctx.Stderr, "error: --suite <id> is required")
		return 2
	}
	trials := 3
	if v := flags.Values["trials"]; v != "" {
		if n, err := strconv.Atoi(v); err == nil && n > 0 {
			trials = n
		}
	}
	provider := strings.TrimSpace(flags.Values["agent"])
	if provider == "" {
		provider = detectAgentProvider()
	}
	if provider != "claude" && provider != "codex" {
		fmt.Fprintln(ctx.Stderr, "error: could not find a supported agent CLI; install `claude` or `codex`, or pass --agent")
		return 1
	}
	if _, err := exec.LookPath(provider); err != nil {
		fmt.Fprintf(ctx.Stderr, "error: agent %q not found on PATH\n", provider)
		return 1
	}

	client, err := newClient(flags)
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}

	// Resolve the skill and ensure we have its content for the treatment arm.
	resolved, err := client.Resolve(ref)
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	if strings.TrimSpace(resolved.InlineContent) == "" && strings.TrimSpace(resolved.RawContentURL) != "" {
		content, fetchErr := client.FetchRawContent(resolved.RawContentURL)
		if fetchErr != nil {
			fmt.Fprintf(ctx.Stderr, "error: %s\n", fetchErr)
			return 1
		}
		resolved.InlineContent = content
	}
	if strings.TrimSpace(resolved.InlineContent) == "" {
		fmt.Fprintln(ctx.Stderr, "error: registry did not provide skill content to evaluate")
		return 1
	}

	suite, err := client.GetSuite(suiteID)
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	verifiers, err := client.FetchVerifiers(suiteID, suite.Version)
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: could not fetch verifiers for suite %s: %s\n", suiteID, err)
		return 1
	}

	cfg := runner.Config{Trials: trials, Model: flags.Values["model"]}

	// Cost estimate + confirmation.
	estTokens, estCost := runner.EstimateCost(suite, cfg)
	if !flags.WantsStructuredOutput() {
		fmt.Fprintf(ctx.Stdout, "Eval plan: skill %s vs no-skill on suite %s@%s\n", resolved.Slug, suite.ID, suite.Version)
		fmt.Fprintf(ctx.Stdout, "  agent: %s | model: %s | %d trials/arm | %d tasks × 2 arms\n",
			provider, orDash(cfg.Model), trials, len(suite.Tasks))
		fmt.Fprintf(ctx.Stdout, "  estimated: ~%s tokens, ~$%.2f on YOUR agent subscription\n", humanInt(estTokens), estCost)
		if !runner.DockerAvailable() {
			fmt.Fprintln(ctx.Stdout, "  note: Docker not detected → worktree isolation; results publish as Self-reported.")
		}
		if !flags.BoolValues["yes"] && !flags.BoolValues["y"] {
			if !confirm(ctx, "Proceed?") {
				fmt.Fprintln(ctx.Stdout, "Aborted.")
				return 1
			}
		}
	}

	agentRunner := &runner.CLIAgentRunner{Provider: provider, Binary: provider, Version: detectAgentVersion(provider)}
	fixtures := runner.NewGitFixtureProvider(suite.Fixture.GitURL, suite.Fixture.Commit)
	defer fixtures.Close()
	verifier := &runner.ScriptVerifier{Commands: verifiers}

	runCtx := context.Background()
	result, err := runner.RunEval(runCtx, suite, resolved, cfg, agentRunner, fixtures, verifier)
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	result.Bundle.CreatedAt = time.Now().UTC().Format(time.RFC3339)

	// Persist the bundle locally regardless of --publish.
	bundlePath, writeErr := writeLocalBundle(result.Bundle.EvalBundle)
	if writeErr != nil {
		fmt.Fprintf(ctx.Stderr, "warning: could not write local bundle: %s\n", writeErr)
	}

	if flags.WantsStructuredOutput() {
		command.WriteOutput(ctx.Stdout, flags, map[string]any{
			"bundle":     result.Bundle.EvalBundle,
			"report":     result.Report,
			"conforming": result.Bundle.Conforming,
			"bundlePath": bundlePath,
		})
	} else {
		printReport(ctx, result)
		if bundlePath != "" {
			fmt.Fprintf(ctx.Stdout, "\nBundle written: %s\n", bundlePath)
		}
	}

	if flags.BoolValues["publish"] {
		resp, pubErr := client.SubmitBundle(result.Bundle.EvalBundle)
		if pubErr != nil {
			fmt.Fprintf(ctx.Stderr, "publish failed: %s\n", pubErr)
			return 1
		}
		if !flags.WantsStructuredOutput() {
			tier := resp.TierState
			if tier == "" {
				tier = "self_reported"
			}
			fmt.Fprintf(ctx.Stdout, "Published (tier: %s%s)\n", tier, conformNote(result.Bundle.Conforming))
		}
	}
	return 0
}

func printReport(ctx command.Context, result runner.Result) {
	fmt.Fprintf(ctx.Stdout, "\nResults (%d trials/arm, %s isolation):\n", result.Report.TrialsPerArm, result.Report.Isolation)
	for _, d := range result.Report.Deltas {
		fmt.Fprintf(ctx.Stdout, "  %-24s pass %.0f%%→%.0f%% (%+.0f pp), tokens %+.1f%%\n",
			d.TaskID, d.ControlPassRate*100, d.TreatmentPassRate*100, d.PassRateDelta*100, d.TokenDeltaPct)
	}
	if result.Report.LowNCaveat {
		fmt.Fprintln(ctx.Stdout, "  (low N: <5 trials/arm — treat deltas as directional, not significant)")
	}
}

func conformNote(conforming bool) string {
	if conforming {
		return ""
	}
	return "; not on reference environment → not eligible for Community-reported aggregation"
}

func writeLocalBundle(bundle reg.EvalBundle) (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	dir := filepath.Join(home, ".skillrank", "bundles")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return "", err
	}
	name := fmt.Sprintf("%s_%s_%d.json", sanitize(bundle.SkillSlug), sanitize(bundle.SuiteID), time.Now().Unix())
	path := filepath.Join(dir, name)
	buf, err := json.MarshalIndent(bundle, "", "  ")
	if err != nil {
		return "", err
	}
	if err := os.WriteFile(path, buf, 0o644); err != nil {
		return "", err
	}
	return path, nil
}

func detectAgentProvider() string {
	if _, err := exec.LookPath("claude"); err == nil {
		return "claude"
	}
	if _, err := exec.LookPath("codex"); err == nil {
		return "codex"
	}
	return ""
}

func detectAgentVersion(provider string) string {
	cmd := exec.Command(provider, "--version")
	out, err := cmd.Output()
	if err != nil {
		return "unknown"
	}
	fields := strings.Fields(strings.TrimSpace(string(out)))
	for _, f := range fields {
		if len(f) > 0 && (f[0] >= '0' && f[0] <= '9') {
			return f
		}
	}
	return strings.TrimSpace(string(out))
}

func orDash(s string) string {
	if strings.TrimSpace(s) == "" {
		return "(agent default)"
	}
	return s
}

func humanInt(n int) string {
	if n >= 1_000_000 {
		return fmt.Sprintf("%.1fM", float64(n)/1_000_000)
	}
	if n >= 1_000 {
		return fmt.Sprintf("%.0fk", float64(n)/1_000)
	}
	return strconv.Itoa(n)
}

func sanitize(s string) string {
	var b strings.Builder
	for _, r := range s {
		switch {
		case r >= 'a' && r <= 'z', r >= 'A' && r <= 'Z', r >= '0' && r <= '9', r == '-', r == '_':
			b.WriteRune(r)
		default:
			b.WriteRune('-')
		}
	}
	return b.String()
}
