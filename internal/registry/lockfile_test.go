package registry

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestLockfileRoundTripPreservesForeignFields(t *testing.T) {
	repoRoot := t.TempDir()
	// Seed a lockfile with a foreign top-level key and a foreign per-entry key,
	// mimicking a file another tool may co-own.
	seed := `{
  "version": 1,
  "toolMeta": {"writtenBy": "some-other-tool"},
  "skills": [
    {"slug": "owner/foreign", "skillPath": ".claude/skills/foreign/SKILL.md", "computedHash": "sha256:aaa", "customField": "keep-me"}
  ]
}`
	if err := os.WriteFile(filepath.Join(repoRoot, LockfileName), []byte(seed), 0o644); err != nil {
		t.Fatal(err)
	}
	lf, err := LoadLockfile(repoRoot)
	if err != nil {
		t.Fatal(err)
	}
	// Add our own entry and save.
	lf.Upsert(LockEntry{
		Slug: "owner/ours", SkillPath: ".claude/skills/ours/SKILL.md",
		ComputedHash: "sha256:bbb", SourceType: "github",
	})
	if err := lf.Save(); err != nil {
		t.Fatal(err)
	}
	// Reload raw JSON and assert foreign fields survived.
	raw, _ := os.ReadFile(filepath.Join(repoRoot, LockfileName))
	var doc map[string]json.RawMessage
	if err := json.Unmarshal(raw, &doc); err != nil {
		t.Fatal(err)
	}
	if _, ok := doc["toolMeta"]; !ok {
		t.Fatal("foreign top-level key toolMeta was dropped")
	}
	var skills []map[string]json.RawMessage
	_ = json.Unmarshal(doc["skills"], &skills)
	foundCustom := false
	for _, s := range skills {
		if _, ok := s["customField"]; ok {
			foundCustom = true
		}
	}
	if !foundCustom {
		t.Fatal("foreign per-entry key customField was dropped")
	}
	if len(skills) != 2 {
		t.Fatalf("expected 2 entries after upsert, got %d", len(skills))
	}
}

func TestLockfileRemove(t *testing.T) {
	repoRoot := t.TempDir()
	lf, _ := LoadLockfile(repoRoot)
	lf.Upsert(LockEntry{Slug: "a", SkillPath: "p/a", ComputedHash: "h"})
	if !lf.Remove("p/a") {
		t.Fatal("expected remove to report true")
	}
	if lf.Remove("p/a") {
		t.Fatal("expected second remove to report false")
	}
}
