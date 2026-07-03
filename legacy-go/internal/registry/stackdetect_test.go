package registry

import (
	"os"
	"path/filepath"
	"testing"
)

func TestDetectStackFromMarkers(t *testing.T) {
	repoRoot := t.TempDir()
	write := func(name, content string) {
		if err := os.WriteFile(filepath.Join(repoRoot, name), []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	write("components.json", `{"style":"default"}`)
	write("package.json", `{"dependencies":{"next":"15.0.0","@playwright/test":"1.50.0"}}`)

	got := DetectStack(repoRoot)
	has := func(stack string) bool {
		for _, s := range got.Stacks {
			if s == stack {
				return true
			}
		}
		return false
	}
	if !has("shadcn") {
		t.Errorf("expected shadcn from components.json; got %v", got.Stacks)
	}
	if !has("nextjs") {
		t.Errorf("expected nextjs from package.json; got %v", got.Stacks)
	}
	if !has("playwright") {
		t.Errorf("expected playwright from package.json; got %v", got.Stacks)
	}
}

func TestDetectStackEmptyRepo(t *testing.T) {
	got := DetectStack(t.TempDir())
	if len(got.Stacks) != 0 {
		t.Fatalf("expected no stacks in empty repo, got %v", got.Stacks)
	}
}
