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

	// --api-url pins the registry the agent's MCP server talks to (e.g. a local
	// `skillrank serve`). It is written into the MCP entry's env so the agent-
	// launched server hits it without any shell setup.
	apiURL := strings.TrimSpace(flags.Values["api-url"])

	if flags.BoolValues["print"] {
		fmt.Fprintf(ctx.Stdout, "Claude Code (%s) — add under \"mcpServers\":\n", claudePath)
		fmt.Fprintf(ctx.Stdout, "  \"skillrank\": %s\n\n", claudeEntryJSON(self, apiURL))
		fmt.Fprintf(ctx.Stdout, "Codex (%s) — append:\n%s\n", codexPath, codexBlock(self, apiURL))
		return 0
	}

	doClaude := !flags.BoolValues["no-claude"]
	doCodex := !flags.BoolValues["no-codex"]
	rc := 0

	if doClaude {
		if err := ensureClaudeMCP(claudePath, self, apiURL); err != nil {
			fmt.Fprintf(ctx.Stderr, "Claude Code: %s\n", err)
			rc = 1
		} else {
			fmt.Fprintf(ctx.Stdout, "✓ Registered skillrank MCP with Claude Code (%s)\n", claudePath)
		}
	}
	if doCodex {
		if err := ensureCodexMCP(codexPath, self, apiURL); err != nil {
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

func codexBlock(self, apiURL string) string {
	block := fmt.Sprintf("[mcp_servers.skillrank]\ncommand = %q\nargs = [\"mcp\"]\n", self)
	if apiURL != "" {
		block += fmt.Sprintf("[mcp_servers.skillrank.env]\nSKILLRANK_API_URL = %q\n", apiURL)
	}
	return block
}

func claudeEntry(self, apiURL string) map[string]any {
	entry := map[string]any{"type": "stdio", "command": self, "args": []string{"mcp"}}
	if apiURL != "" {
		entry["env"] = map[string]any{"SKILLRANK_API_URL": apiURL}
	}
	return entry
}

func claudeEntryJSON(self, apiURL string) string {
	buf, _ := json.Marshal(claudeEntry(self, apiURL))
	return string(buf)
}

// ensureClaudeMCP merges an mcpServers.skillrank entry into ~/.claude.json,
// preserving all other data. Backs up the file first.
func ensureClaudeMCP(path, self, apiURL string) error {
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
	servers["skillrank"] = claudeEntry(self, apiURL)
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

// ensureCodexMCP writes the [mcp_servers.skillrank] block to config.toml,
// replacing any prior skillrank block (so re-running updates it) and preserving
// everything else. TOML is edited textually to avoid a TOML dependency.
func ensureCodexMCP(path, self, apiURL string) error {
	var existing string
	if data, err := os.ReadFile(path); err == nil {
		existing = stripCodexSkillrankBlock(string(data))
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
	b.WriteString(strings.TrimRight(existing, "\n"))
	if strings.TrimSpace(existing) != "" {
		b.WriteString("\n\n")
	}
	b.WriteString(codexBlock(self, apiURL))
	return os.WriteFile(path, []byte(b.String()), 0o644)
}

// stripCodexSkillrankBlock removes any [mcp_servers.skillrank] and
// [mcp_servers.skillrank.env] tables (from each header to the next table header
// or EOF), leaving all other config intact.
func stripCodexSkillrankBlock(s string) string {
	lines := strings.Split(s, "\n")
	var out []string
	skipping := false
	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "[") {
			// A new table header: skip skillrank tables, keep others.
			skipping = trimmed == "[mcp_servers.skillrank]" || trimmed == "[mcp_servers.skillrank.env]"
		}
		if !skipping {
			out = append(out, line)
		}
	}
	return strings.Join(out, "\n")
}

func backup(path string, data []byte) error {
	return os.WriteFile(path+".skillrank-bak", data, 0o644)
}
