package skillscmd

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/buildbetter/skillrank/internal/command"
)

// runSetup registers the skillrank MCP server with Claude Code and Codex so that,
// after this one-time step, "find me a skill for playwright" works in either agent
// with no command knowledge — skillrank's tools are in the agent's vocabulary.
//
// It writes directly to the agents' config files (idempotent, backed up) rather
// than shelling out, so it works even if the agent CLIs are not on PATH.
func runSetup(args []string, ctx command.Context) int {
	flags := command.ParseFlags(args)
	self := selfPath()

	claudePath := flags.Values["claude-config"]
	if claudePath == "" {
		claudePath = defaultClaudeConfigPath()
	}
	codexPath := flags.Values["codex-config"]
	if codexPath == "" {
		codexPath = defaultCodexConfigPath()
	}

	if flags.BoolValues["print"] {
		fmt.Fprintf(ctx.Stdout, "Claude Code (%s) — add under \"mcpServers\":\n", claudePath)
		fmt.Fprintf(ctx.Stdout, "  \"skillrank\": {\"type\": \"stdio\", \"command\": %q, \"args\": [\"mcp\"]}\n\n", self)
		fmt.Fprintf(ctx.Stdout, "Codex (%s) — append:\n%s\n", codexPath, codexBlock(self))
		return 0
	}

	doClaude := !flags.BoolValues["no-claude"]
	doCodex := !flags.BoolValues["no-codex"]
	rc := 0

	if doClaude {
		if err := ensureClaudeMCP(claudePath, self); err != nil {
			fmt.Fprintf(ctx.Stderr, "Claude Code: %s\n", err)
			rc = 1
		} else {
			fmt.Fprintf(ctx.Stdout, "✓ Registered skillrank MCP with Claude Code (%s)\n", claudePath)
		}
	}
	if doCodex {
		if err := ensureCodexMCP(codexPath, self); err != nil {
			fmt.Fprintf(ctx.Stderr, "Codex: %s\n", err)
			rc = 1
		} else {
			fmt.Fprintf(ctx.Stdout, "✓ Registered skillrank MCP with Codex (%s)\n", codexPath)
		}
	}

	if rc == 0 {
		fmt.Fprintln(ctx.Stdout, "\nDone. Restart your agent, then just ask it to find, install, or evaluate skills —")
		fmt.Fprintln(ctx.Stdout, "no commands to remember. (Claude Code prompts once to approve the tools; approve them.)")
		fmt.Fprintln(ctx.Stdout, "To skip the prompt, add to ~/.claude/settings.json: {\"permissions\":{\"allow\":[\"mcp__skillrank\"]}}")
	}
	return rc
}

// selfPath returns the absolute path to this binary, falling back to the bare
// name "skillrank" (assumed on PATH) if resolution fails.
func selfPath() string {
	exe, err := os.Executable()
	if err != nil {
		return "skillrank"
	}
	if resolved, err := filepath.EvalSymlinks(exe); err == nil {
		return resolved
	}
	return exe
}

func defaultClaudeConfigPath() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return ".claude.json"
	}
	return filepath.Join(home, ".claude.json")
}

func defaultCodexConfigPath() string {
	if h := strings.TrimSpace(os.Getenv("CODEX_HOME")); h != "" {
		return filepath.Join(h, "config.toml")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return filepath.Join(".codex", "config.toml")
	}
	return filepath.Join(home, ".codex", "config.toml")
}

func codexBlock(self string) string {
	return fmt.Sprintf("[mcp_servers.skillrank]\ncommand = %q\nargs = [\"mcp\"]\n", self)
}

// ensureClaudeMCP merges an mcpServers.skillrank entry into ~/.claude.json,
// preserving all other data. Backs up the file first.
func ensureClaudeMCP(path, self string) error {
	doc := map[string]any{}
	if data, err := os.ReadFile(path); err == nil {
		if len(strings.TrimSpace(string(data))) > 0 {
			if err := json.Unmarshal(data, &doc); err != nil {
				return fmt.Errorf("parse %s: %w", path, err)
			}
		}
		if err := backup(path, data); err != nil {
			return err
		}
	} else if !os.IsNotExist(err) {
		return err
	}

	servers, _ := doc["mcpServers"].(map[string]any)
	if servers == nil {
		servers = map[string]any{}
	}
	servers["skillrank"] = map[string]any{
		"type":    "stdio",
		"command": self,
		"args":    []string{"mcp"},
	}
	doc["mcpServers"] = servers

	out, err := json.MarshalIndent(doc, "", "  ")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	return os.WriteFile(path, append(out, '\n'), 0o644)
}

// ensureCodexMCP appends a [mcp_servers.skillrank] block to config.toml when
// absent (idempotent). TOML is append-only edited to avoid a TOML dependency and
// to preserve the user's existing config exactly.
func ensureCodexMCP(path, self string) error {
	var existing string
	if data, err := os.ReadFile(path); err == nil {
		existing = string(data)
		if strings.Contains(existing, "[mcp_servers.skillrank]") {
			return nil // already registered
		}
		if err := backup(path, data); err != nil {
			return err
		}
	} else if !os.IsNotExist(err) {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	var b strings.Builder
	b.WriteString(existing)
	if existing != "" && !strings.HasSuffix(existing, "\n") {
		b.WriteString("\n")
	}
	if existing != "" {
		b.WriteString("\n")
	}
	b.WriteString(codexBlock(self))
	return os.WriteFile(path, []byte(b.String()), 0o644)
}

func backup(path string, data []byte) error {
	return os.WriteFile(path+".skillrank-bak", data, 0o644)
}
