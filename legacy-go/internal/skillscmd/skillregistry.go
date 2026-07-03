// Package skillregistry implements the `bb skills` command family and the
// standalone `skillrank` binary's command surface. One implementation, two
// mount points (see cmd/skillrank/main.go and internal/cli/router.go).
package skillscmd

import (
	"bufio"
	"fmt"
	"sort"
	"strconv"
	"strings"

	"github.com/buildbetter/skillrank/internal/command"
	reg "github.com/buildbetter/skillrank/internal/registry"
)

// Run dispatches a skills subcommand. args[0] is the subcommand name.
func Run(args []string, ctx command.Context) int {
	if len(args) == 0 {
		printUsage(ctx)
		return 0
	}
	sub := args[0]
	tail := args[1:]
	switch sub {
	case "help", "--help", "-h":
		printUsage(ctx)
		return 0
	case "search":
		return runSearch(tail, ctx)
	case "show":
		return runShow(tail, ctx)
	case "install", "add":
		return runInstall(tail, ctx)
	case "list", "ls":
		return runList(tail, ctx)
	case "uninstall", "remove", "rm":
		return runUninstall(tail, ctx)
	case "recommend":
		return runRecommend(tail, ctx)
	case "eval":
		return runEval(tail, ctx)
	case "skill":
		return runSkill(tail, ctx)
	case "mcp":
		return runMCP(tail, ctx)
	case "setup":
		return runSetup(tail, ctx)
	case "serve":
		return runServe(tail, ctx)
	default:
		fmt.Fprintf(ctx.Stderr, "unknown skills subcommand %q\n", sub)
		printUsage(ctx)
		return 2
	}
}

func printUsage(ctx command.Context) {
	fmt.Fprint(ctx.Stdout, `skillrank — find, install, evaluate, and publish agent skills

Open source. Works on its own; the core (search, install, local eval) needs no
account. Integrates with BuildBetter ZeroShot when it is also installed
(equivalently available as `+"`bb skills <command>`"+`).

Usage:
  skillrank <command> [flags]

Commands:
  search <query>     Search the public skill registry.
  show <ref>         Show a skill's scores, security, and eval results.
  install <ref>      Install a skill into this repo (hash-verified).
  list               List installed skills and drift.
  uninstall <slug>   Remove an installed skill.
  recommend          Suggest skills for this repo's detected stack.
  eval <ref>         Run a local paired eval and optionally publish results.
  skill [--install]  Print, or install into .claude/skills, the SKILL.md that
                     teaches your agent (Claude Code/Codex) to use skillrank.
  setup              Register the skillrank MCP server with Claude Code and Codex
                     so the agent uses skillrank automatically (one-time).
  mcp                Run as an MCP stdio server (invoked by the agent; not by you).
  serve [--port N]   Run a local registry server (seed catalog) so search/install
                     work with no hosted backend. Set SKILLRANK_API_URL to it.

Global flags:
  --json             Emit JSON.
  --api-base-url URL Override the registry API base URL.
`)
}

func newClient(flags command.Flags) (reg.Client, error) {
	return reg.NewClient(flags.Values["api-base-url"], nil)
}

func runSearch(args []string, ctx command.Context) int {
	flags := command.ParseFlags(args)
	query := strings.Join(flags.Positionals, " ")
	client, err := newClient(flags)
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	limit := 20
	if v := flags.Values["limit"]; v != "" {
		if n, convErr := strconv.Atoi(v); convErr == nil && n > 0 {
			limit = n
		}
	}
	resp, err := client.Search(reg.SearchOptions{
		Query:    query,
		Stack:    flags.Values["stack"],
		Agent:    flags.Values["agent"],
		Category: flags.Values["category"],
		Sort:     flags.Values["sort"],
		Limit:    limit,
	})
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	if flags.WantsStructuredOutput() {
		command.WriteOutput(ctx.Stdout, flags, resp)
		return 0
	}
	if len(resp.Items) == 0 {
		fmt.Fprintln(ctx.Stdout, "No skills matched.")
		return 0
	}
	for _, item := range resp.Items {
		rating := "—"
		if item.RatingAverage != nil {
			rating = fmt.Sprintf("%.1f★ (%d)", *item.RatingAverage, item.RatingCount)
		}
		fmt.Fprintf(ctx.Stdout, "%-32s %-10s scan:%-7s %s\n", item.Slug, tierShort(item.ScanTier), item.ScanTier, rating)
		if item.Summary != "" {
			fmt.Fprintf(ctx.Stdout, "    %s\n", truncate(item.Summary, 100))
		}
	}
	if resp.NextCursor != "" {
		fmt.Fprintf(ctx.Stdout, "\n(more results — %d total)\n", resp.Total)
	}
	return 0
}

func runShow(args []string, ctx command.Context) int {
	flags := command.ParseFlags(args)
	if len(flags.Positionals) == 0 {
		fmt.Fprintln(ctx.Stderr, "usage: show <ref>")
		return 2
	}
	slug, _ := reg.SplitRef(flags.Positionals[0])
	client, err := newClient(flags)
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	detail, err := client.Show(slug)
	if err != nil {
		if reg.IsNotFound(err) {
			fmt.Fprintf(ctx.Stderr, "skill %q not found\n", slug)
			return 1
		}
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	if flags.WantsStructuredOutput() {
		command.WriteOutput(ctx.Stdout, flags, detail)
		return 0
	}
	fmt.Fprintf(ctx.Stdout, "%s\n", detail.Slug)
	if detail.Summary != "" {
		fmt.Fprintf(ctx.Stdout, "  %s\n", detail.Summary)
	}
	fmt.Fprintf(ctx.Stdout, "  source: %s (%s)\n", detail.SourceURL, detail.SourceType)
	fmt.Fprintf(ctx.Stdout, "  scan:   %s\n", detail.ScanTier)
	if len(detail.Stacks) > 0 {
		fmt.Fprintf(ctx.Stdout, "  stacks: %s\n", strings.Join(detail.Stacks, ", "))
	}
	if len(detail.EvalCells) == 0 {
		fmt.Fprintln(ctx.Stdout, "  evals:  none yet")
	} else {
		fmt.Fprintln(ctx.Stdout, "  evals:")
		for _, cell := range detail.EvalCells {
			lift := "—"
			if cell.SuccessLiftPct != nil {
				lift = fmt.Sprintf("%+.1f%%", *cell.SuccessLiftPct)
			}
			tok := "—"
			if cell.NetTokenDeltaPct != nil {
				tok = fmt.Sprintf("%+.1f%%", *cell.NetTokenDeltaPct)
			}
			label := string(cell.Tier)
			if cell.Gated {
				label += " (gated)"
			}
			fmt.Fprintf(ctx.Stdout, "    [%s] %s/%s on %s: lift %s, tokens %s (n=%d accts, %d trials)\n",
				label, cell.Agent, cell.Model, cell.Suite, lift, tok, cell.NAccounts, cell.NTrials)
		}
	}
	return 0
}

func runInstall(args []string, ctx command.Context) int {
	flags := command.ParseFlags(args)
	if len(flags.Positionals) == 0 {
		fmt.Fprintln(ctx.Stderr, "usage: install <ref> [--surface DIR] [--yes]")
		return 2
	}
	client, err := newClient(flags)
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	repoRoot := reg.RepoRoot(flags.Values["cwd"])
	ref := flags.Positionals[0]

	// Pre-flight: resolve to show the scan tier before writing.
	resolved, err := client.Resolve(ref)
	if err != nil {
		if reg.IsNotFound(err) {
			fmt.Fprintf(ctx.Stderr, "skill %q not found in the registry\n", ref)
			return 1
		}
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	yes := flags.BoolValues["yes"] || flags.BoolValues["y"]
	if !reg.SafeScanTier(resolved.ScanTier) && !yes {
		fmt.Fprintf(ctx.Stdout, "⚠ %s has scan tier %q (not verified safe).\n", resolved.Slug, resolved.ScanTier)
		if !confirm(ctx, "Install anyway?") {
			fmt.Fprintln(ctx.Stdout, "Aborted.")
			return 1
		}
	} else if !reg.SafeScanTier(resolved.ScanTier) && yes {
		fmt.Fprintf(ctx.Stderr, "warning: installing %s despite scan tier %q (--yes)\n", resolved.Slug, resolved.ScanTier)
	}

	result, err := client.Install(reg.InstallOptions{
		Ref:             ref,
		RepoRoot:        repoRoot,
		SurfaceOverride: flags.Values["surface"],
	})
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	if flags.WantsStructuredOutput() {
		command.WriteOutput(ctx.Stdout, flags, result)
		return 0
	}
	if result.AlreadyExact {
		fmt.Fprintf(ctx.Stdout, "%s already installed at %s (up to date).\n", result.Slug, result.SkillPath)
		return 0
	}
	fmt.Fprintf(ctx.Stdout, "Installed %s → %s (scan: %s)\n", result.Slug, result.SkillPath, result.ScanTier)
	return 0
}

func runList(args []string, ctx command.Context) int {
	flags := command.ParseFlags(args)
	repoRoot := reg.RepoRoot(flags.Values["cwd"])
	rows, err := reg.ListInstalled(repoRoot)
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	if flags.WantsStructuredOutput() {
		command.WriteOutput(ctx.Stdout, flags, rows)
		return 0
	}
	if len(rows) == 0 {
		fmt.Fprintln(ctx.Stdout, "No registry-installed skills in this repo.")
		return 0
	}
	sort.SliceStable(rows, func(i, j int) bool { return rows[i].Slug < rows[j].Slug })
	for _, r := range rows {
		fmt.Fprintf(ctx.Stdout, "%-32s %-16s %s\n", r.Slug, r.State, r.SkillPath)
	}
	return 0
}

func runUninstall(args []string, ctx command.Context) int {
	flags := command.ParseFlags(args)
	if len(flags.Positionals) == 0 {
		fmt.Fprintln(ctx.Stderr, "usage: uninstall <slug> [--yes]")
		return 2
	}
	slug := flags.Positionals[0]
	if !flags.BoolValues["yes"] && !flags.BoolValues["y"] {
		if !confirm(ctx, fmt.Sprintf("Remove skill %q and its files?", slug)) {
			fmt.Fprintln(ctx.Stdout, "Aborted.")
			return 1
		}
	}
	repoRoot := reg.RepoRoot(flags.Values["cwd"])
	path, err := reg.Uninstall(repoRoot, slug)
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	fmt.Fprintf(ctx.Stdout, "Removed %s (%s)\n", slug, path)
	return 0
}

func runRecommend(args []string, ctx command.Context) int {
	flags := command.ParseFlags(args)
	repoRoot := reg.RepoRoot(flags.Values["cwd"])
	detected := reg.DetectStack(repoRoot)
	client, err := newClient(flags)
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	type recommendation struct {
		Detected reg.DetectedStack   `json:"detected"`
		Skills   []reg.SkillSummary  `json:"skills"`
	}
	rec := recommendation{Detected: detected}
	seen := map[string]bool{}
	for _, stack := range detected.Stacks {
		resp, searchErr := client.Search(reg.SearchOptions{Stack: stack, Sort: "signals", Limit: 5})
		if searchErr != nil {
			continue
		}
		for _, item := range resp.Items {
			if seen[item.Slug] {
				continue
			}
			seen[item.Slug] = true
			rec.Skills = append(rec.Skills, item)
		}
	}
	if flags.WantsStructuredOutput() {
		command.WriteOutput(ctx.Stdout, flags, rec)
		return 0
	}
	if len(detected.Stacks) == 0 {
		fmt.Fprintln(ctx.Stdout, "Could not detect a stack in this repo. Try `skills search <query>`.")
		return 0
	}
	fmt.Fprintf(ctx.Stdout, "Detected stack: %s\n", strings.Join(detected.Stacks, ", "))
	if len(rec.Skills) == 0 {
		fmt.Fprintln(ctx.Stdout, "No matching skills found in the registry yet.")
		return 0
	}
	fmt.Fprintln(ctx.Stdout, "Recommended skills:")
	for _, item := range rec.Skills {
		fmt.Fprintf(ctx.Stdout, "  %-32s scan:%-7s  %s\n", item.Slug, item.ScanTier, truncate(item.Summary, 80))
	}
	fmt.Fprintln(ctx.Stdout, "\nInstall one with: skillrank install <slug>")
	return 0
}

func confirm(ctx command.Context, prompt string) bool {
	fmt.Fprintf(ctx.Stdout, "%s [y/N] ", prompt)
	reader := bufio.NewReader(ctx.Stdin)
	line, _ := reader.ReadString('\n')
	answer := strings.ToLower(strings.TrimSpace(line))
	return answer == "y" || answer == "yes"
}

func tierShort(tier reg.ScanTier) string {
	switch tier {
	case reg.ScanSafe:
		return "safe"
	case reg.ScanFlagged, reg.ScanHigh:
		return "RISK"
	default:
		return string(tier)
	}
}

func truncate(s string, n int) string {
	s = strings.TrimSpace(strings.ReplaceAll(s, "\n", " "))
	if len(s) <= n {
		return s
	}
	return s[:n-1] + "…"
}
