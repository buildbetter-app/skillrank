package skillscmd

import (
	_ "embed"
	"fmt"
	"os"
	"path/filepath"

	"github.com/buildbetter/skillrank/internal/command"
	reg "github.com/buildbetter/skillrank/internal/registry"
)

// skillrankSkillMarkdown is the SKILL.md that teaches an agent (Claude Code,
// Codex, etc.) when and how to drive the skillrank CLI. Embedding it means the
// installed binary can write it into any repo with `skillrank skill --install`.
//
//go:embed skillrank_skill.md
var skillrankSkillMarkdown string

// runSkill prints or installs the skillrank usage skill. With --install it writes
// the skill into this repo's skill surface so the agent will use skillrank
// automatically; otherwise it prints the SKILL.md to stdout.
func runSkill(args []string, ctx command.Context) int {
	flags := command.ParseFlags(args)
	if !flags.BoolValues["install"] {
		fmt.Fprint(ctx.Stdout, skillrankSkillMarkdown)
		if !flags.WantsStructuredOutput() {
			fmt.Fprintln(ctx.Stderr, "\n(Run `skillrank skill --install` to add this to .claude/skills so your agent uses skillrank automatically.)")
		}
		return 0
	}

	repoRoot := reg.RepoRoot(flags.Values["cwd"])
	surfaceRel, surfaceAbs, err := reg.ResolveSurface(repoRoot, flags.Values["surface"])
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	dir := filepath.Join(surfaceAbs, "skillrank")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	path := filepath.Join(dir, "SKILL.md")
	if err := os.WriteFile(path, []byte(skillrankSkillMarkdown), 0o644); err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	rel := filepath.ToSlash(filepath.Join(surfaceRel, "skillrank", "SKILL.md"))
	if flags.WantsStructuredOutput() {
		command.WriteOutput(ctx.Stdout, flags, map[string]string{"skillPath": rel})
		return 0
	}
	fmt.Fprintf(ctx.Stdout, "Installed the skillrank skill → %s\n", rel)
	fmt.Fprintln(ctx.Stdout, "Your agent will now use skillrank automatically when you ask it to find, install, or evaluate skills.")
	return 0
}
