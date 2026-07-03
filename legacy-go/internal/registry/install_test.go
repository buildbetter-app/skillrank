package registry

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func resolveServer(t *testing.T, resp ResolveResponse) *httptest.Server {
	t.Helper()
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/resolve") {
			// Public read must not carry an Authorization header.
			if r.Header.Get("Authorization") != "" {
				t.Errorf("public resolve read carried an Authorization header")
			}
			w.Header().Set("Content-Type", "application/json")
			_ = json.NewEncoder(w).Encode(resp)
			return
		}
		http.NotFound(w, r)
	}))
}

func TestInstallWritesSkillAndLockfileOnHashMatch(t *testing.T) {
	content := "---\nname: example-skill\ndescription: test\n---\nDo the thing.\n"
	resp := ResolveResponse{
		Slug:          "owner/example-skill",
		Version:       "sha256:abc",
		SourceType:    "github",
		SourceURL:     "https://github.com/owner/example-skill",
		ContentHash:   ComputeContentHash(content),
		ScanTier:      ScanSafe,
		InlineContent: content,
	}
	server := resolveServer(t, resp)
	defer server.Close()

	repoRoot := t.TempDir()
	client, err := NewClient(server.URL, nil)
	if err != nil {
		t.Fatal(err)
	}
	result, err := client.Install(InstallOptions{Ref: "owner/example-skill", RepoRoot: repoRoot, NowRFC3339: "2026-07-01T00:00:00Z"})
	if err != nil {
		t.Fatalf("install failed: %v", err)
	}
	// The skill file must exist with the exact content.
	skillFile := filepath.Join(repoRoot, filepath.FromSlash(result.SkillPath))
	got, err := os.ReadFile(skillFile)
	if err != nil {
		t.Fatalf("skill file not written: %v", err)
	}
	if string(got) != content {
		t.Fatalf("skill content mismatch:\n%q", string(got))
	}
	// The lockfile must record the install.
	lf, err := LoadLockfile(repoRoot)
	if err != nil {
		t.Fatal(err)
	}
	if lf.FindBySlug("owner/example-skill") == nil {
		t.Fatal("lockfile entry not recorded")
	}
	// A second identical install is idempotent.
	again, err := client.Install(InstallOptions{Ref: "owner/example-skill", RepoRoot: repoRoot})
	if err != nil || !again.AlreadyExact {
		t.Fatalf("expected idempotent second install, got %+v err=%v", again, err)
	}
}

func TestInstallRefusesOnHashMismatchAndWritesNothing(t *testing.T) {
	content := "---\nname: tampered\n---\nreal content\n"
	resp := ResolveResponse{
		Slug:          "owner/tampered",
		Version:       "sha256:xyz",
		SourceType:    "github",
		ContentHash:   "sha256:deadbeefdeadbeef", // deliberately wrong
		ScanTier:      ScanSafe,
		InlineContent: content,
	}
	server := resolveServer(t, resp)
	defer server.Close()

	repoRoot := t.TempDir()
	client, _ := NewClient(server.URL, nil)
	_, err := client.Install(InstallOptions{Ref: "owner/tampered", RepoRoot: repoRoot})
	if err == nil {
		t.Fatal("expected hash-mismatch refusal, got nil error")
	}
	if !strings.Contains(err.Error(), "content hash mismatch") {
		t.Fatalf("expected content hash mismatch error, got: %v", err)
	}
	// No skill file and no lockfile should have been written.
	if _, statErr := os.Stat(filepath.Join(repoRoot, LockfileName)); statErr == nil {
		t.Fatal("lockfile written despite refusal")
	}
	entries, _ := os.ReadDir(filepath.Join(repoRoot, ".claude", "skills"))
	if len(entries) != 0 {
		t.Fatalf("skill files written despite refusal: %v", entries)
	}
}

func TestInstallFailsClosedOnTombstone(t *testing.T) {
	resp := ResolveResponse{Slug: "owner/gone", Tombstoned: true, TombstoneReason: "malware takedown"}
	server := resolveServer(t, resp)
	defer server.Close()
	client, _ := NewClient(server.URL, nil)
	_, err := client.Install(InstallOptions{Ref: "owner/gone", RepoRoot: t.TempDir()})
	if err == nil || !strings.Contains(err.Error(), "malware takedown") {
		t.Fatalf("expected tombstone refusal, got: %v", err)
	}
}
