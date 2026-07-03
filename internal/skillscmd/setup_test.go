package skillscmd

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestEnsureCodexMCPPreservesAndIsIdempotent(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.toml")
	existing := "[mcp_servers.playwright]\ncommand = \"npx\"\nargs = [\"@playwright/mcp@latest\"]\n"
	if err := os.WriteFile(path, []byte(existing), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := ensureCodexMCP(path, "/usr/local/bin/skillrank", ""); err != nil {
		t.Fatal(err)
	}
	data, _ := os.ReadFile(path)
	s := string(data)
	if !strings.Contains(s, "[mcp_servers.playwright]") {
		t.Fatal("existing playwright server was lost")
	}
	if !strings.Contains(s, "[mcp_servers.skillrank]") || !strings.Contains(s, "/usr/local/bin/skillrank") {
		t.Fatal("skillrank server not appended")
	}
	// Idempotent: second run adds no duplicate.
	if err := ensureCodexMCP(path, "/usr/local/bin/skillrank", ""); err != nil {
		t.Fatal(err)
	}
	data, _ = os.ReadFile(path)
	if n := strings.Count(string(data), "[mcp_servers.skillrank]"); n != 1 {
		t.Fatalf("expected exactly one skillrank section, got %d", n)
	}
}

func TestEnsureClaudeMCPMergesPreservingData(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "claude.json")
	if err := os.WriteFile(path, []byte(`{"numStartups":42,"mcpServers":{"context7":{"command":"npx","args":["-y","context7"]}}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := ensureClaudeMCP(path, "/usr/local/bin/skillrank", ""); err != nil {
		t.Fatal(err)
	}
	var doc map[string]any
	data, _ := os.ReadFile(path)
	if err := json.Unmarshal(data, &doc); err != nil {
		t.Fatal(err)
	}
	if doc["numStartups"].(float64) != 42 {
		t.Fatal("numStartups lost")
	}
	servers := doc["mcpServers"].(map[string]any)
	if _, ok := servers["context7"]; !ok {
		t.Fatal("context7 server lost")
	}
	sr, ok := servers["skillrank"].(map[string]any)
	if !ok {
		t.Fatal("skillrank server not added")
	}
	if sr["command"] != "/usr/local/bin/skillrank" || sr["type"] != "stdio" {
		t.Fatalf("unexpected skillrank entry: %v", sr)
	}
	// A .bak backup must exist.
	if _, err := os.Stat(path + ".skillrank-bak"); err != nil {
		t.Fatal("expected backup file")
	}
}

func TestEnsureClaudeMCPCreatesFileWhenMissing(t *testing.T) {
	path := filepath.Join(t.TempDir(), "claude.json")
	if err := ensureClaudeMCP(path, "skillrank", ""); err != nil {
		t.Fatal(err)
	}
	var doc map[string]any
	data, _ := os.ReadFile(path)
	_ = json.Unmarshal(data, &doc)
	if _, ok := doc["mcpServers"].(map[string]any)["skillrank"]; !ok {
		t.Fatal("skillrank not added to fresh config")
	}
}
