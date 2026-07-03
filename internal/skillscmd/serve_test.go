package skillscmd

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	reg "github.com/buildbetter/skillrank/internal/registry"
)

func testServer(t *testing.T) *registryServer {
	t.Helper()
	var entries []catalogEntry
	if err := json.Unmarshal(seedCatalogJSON, &entries); err != nil {
		t.Fatalf("seed catalog does not parse: %v", err)
	}
	if len(entries) == 0 {
		t.Fatal("seed catalog is empty")
	}
	index := map[string]*catalogEntry{}
	for i := range entries {
		entries[i].hash = reg.ComputeContentHash(entries[i].Content)
		index[entries[i].Slug] = &entries[i]
	}
	return &registryServer{entries: entries, index: index}
}

func TestServeSearchFrontEndExcludesBackend(t *testing.T) {
	s := testServer(t)
	rec := httptest.NewRecorder()
	s.handleSearch(rec, httptest.NewRequest(http.MethodGet, reg.PathPrefix+"/skills?q=front+end", nil))
	var resp reg.SearchResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &resp); err != nil {
		t.Fatal(err)
	}
	if len(resp.Items) == 0 {
		t.Fatal("expected front-end results")
	}
	for _, item := range resp.Items {
		if item.Category == "backend" {
			t.Errorf("front-end search returned a backend skill: %s", item.Slug)
		}
	}
}

func TestServeResolveReturnsVerifiableContent(t *testing.T) {
	s := testServer(t)
	slug := s.entries[0].Slug
	rec := httptest.NewRecorder()
	s.handleSkill(rec, httptest.NewRequest(http.MethodGet, reg.PathPrefix+"/skills/"+slug+"/resolve", nil))
	var resolved reg.ResolveResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &resolved); err != nil {
		t.Fatal(err)
	}
	// The advertised content hash must match the inline content — this is exactly
	// what `install` verifies, so a mismatch here would break install.
	if !reg.HashesEqual(reg.ComputeContentHash(resolved.InlineContent), resolved.ContentHash) {
		t.Fatalf("resolve content hash does not match its own content for %s", slug)
	}
	if resolved.ScanTier != reg.ScanSafe {
		t.Errorf("expected safe scan tier, got %s", resolved.ScanTier)
	}
}

func TestServeStackFilter(t *testing.T) {
	s := testServer(t)
	rec := httptest.NewRecorder()
	s.handleSearch(rec, httptest.NewRequest(http.MethodGet, reg.PathPrefix+"/skills?stack=go", nil))
	var resp reg.SearchResponse
	_ = json.Unmarshal(rec.Body.Bytes(), &resp)
	for _, item := range resp.Items {
		found := false
		for _, st := range item.Stacks {
			if st == "go" {
				found = true
			}
		}
		if !found {
			t.Errorf("stack=go returned non-go skill %s (%v)", item.Slug, item.Stacks)
		}
	}
}

func TestSetupInjectsApiURLEnv(t *testing.T) {
	// --api-url must land in the MCP config env for both agents.
	claudeEntry := claudeEntry("skillrank", "http://localhost:8899")
	env, ok := claudeEntry["env"].(map[string]any)
	if !ok || env["SKILLRANK_API_URL"] != "http://localhost:8899" {
		t.Fatalf("claude entry missing api-url env: %v", claudeEntry)
	}
	codex := codexBlock("skillrank", "http://localhost:8899")
	if !strings.Contains(codex, "[mcp_servers.skillrank.env]") || !strings.Contains(codex, "http://localhost:8899") {
		t.Fatalf("codex block missing env: %s", codex)
	}
}
