package skillscmd

import (
	_ "embed"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"sort"
	"strconv"
	"strings"

	"github.com/buildbetter/skillrank/internal/command"
	reg "github.com/buildbetter/skillrank/internal/registry"
)

//go:embed seed_catalog.json
var seedCatalogJSON []byte

// catalogEntry is one skill in the local registry catalog.
type catalogEntry struct {
	Slug        string   `json:"slug"`
	DisplayName string   `json:"display_name"`
	Category    string   `json:"category"`
	Stacks      []string `json:"stacks"`
	SourceURL   string   `json:"source_url"`
	Summary     string   `json:"summary"`
	Content     string   `json:"content"`
	hash        string   // computed content hash
}

// runServe starts a local registry server implementing the read half of the
// /v3/rest/skill-registry contract, backed by a seed catalog (embedded, or a
// --catalog file). Point the CLI/MCP at it with SKILLRANK_API_URL=http://host:port.
// This is a real backend you run with one command — the same wire contract the
// hosted registry serves — so search, recommend, show, and install all work
// end-to-end with no external dependency.
func runServe(args []string, ctx command.Context) int {
	flags := command.ParseFlags(args)
	port := 8899
	if v := flags.Values["port"]; v != "" {
		if n, err := strconv.Atoi(v); err == nil {
			port = n
		}
	}

	raw := seedCatalogJSON
	if path := flags.Values["catalog"]; path != "" {
		data, err := os.ReadFile(path)
		if err != nil {
			fmt.Fprintf(ctx.Stderr, "skillrank serve: read catalog: %s\n", err)
			return 1
		}
		raw = data
	}
	var entries []catalogEntry
	if err := json.Unmarshal(raw, &entries); err != nil {
		fmt.Fprintf(ctx.Stderr, "skillrank serve: parse catalog: %s\n", err)
		return 1
	}
	index := map[string]*catalogEntry{}
	for i := range entries {
		entries[i].hash = reg.ComputeContentHash(entries[i].Content)
		index[entries[i].Slug] = &entries[i]
	}

	srv := &registryServer{entries: entries, index: index}
	mux := http.NewServeMux()
	mux.HandleFunc(reg.PathPrefix+"/skills", srv.handleSearch)
	mux.HandleFunc(reg.PathPrefix+"/skills/", srv.handleSkill)
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, _ *http.Request) { _, _ = w.Write([]byte("ok")) })

	addr := fmt.Sprintf(":%d", port)
	fmt.Fprintf(ctx.Stdout, "skillrank registry serving %d skills on http://localhost%s\n", len(entries), addr)
	fmt.Fprintf(ctx.Stdout, "Point your CLI/agent at it:  export SKILLRANK_API_URL=http://localhost%s\n", addr)
	if err := http.ListenAndServe(addr, mux); err != nil {
		fmt.Fprintf(ctx.Stderr, "skillrank serve: %s\n", err)
		return 1
	}
	return 0
}

type registryServer struct {
	entries []catalogEntry
	index   map[string]*catalogEntry
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}

func (s *registryServer) handleSearch(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	query := strings.ToLower(strings.TrimSpace(q.Get("q")))
	stack := strings.ToLower(strings.TrimSpace(q.Get("stack")))
	category := strings.ToLower(strings.TrimSpace(q.Get("category")))
	limit := 20
	if n, err := strconv.Atoi(q.Get("limit")); err == nil && n > 0 {
		limit = n
	}

	var items []reg.SkillSummary
	for i := range s.entries {
		e := &s.entries[i]
		if stack != "" && !containsFold(e.Stacks, stack) {
			continue
		}
		if category != "" && !strings.EqualFold(e.Category, category) {
			continue
		}
		if query != "" && !matchesQuery(e, query) {
			continue
		}
		items = append(items, e.toSummary())
	}
	sort.SliceStable(items, func(i, j int) bool { return items[i].Slug < items[j].Slug })
	if len(items) > limit {
		items = items[:limit]
	}
	writeJSON(w, http.StatusOK, reg.SearchResponse{Items: items, Total: len(items)})
}

func (s *registryServer) handleSkill(w http.ResponseWriter, r *http.Request) {
	// The slug may contain slashes (owner/name); parse it out of the decoded path
	// rather than relying on a path-segment router (which %2F-encoded slugs break).
	rest := strings.TrimPrefix(r.URL.Path, reg.PathPrefix+"/skills/")
	if resolveSlug, ok := strings.CutSuffix(rest, "/resolve"); ok {
		s.handleResolve(w, resolveSlug)
		return
	}
	s.handleShow(w, rest)
}

func (s *registryServer) handleShow(w http.ResponseWriter, slug string) {
	e, ok := s.index[slug]
	if !ok {
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "not found"})
		return
	}
	detail := reg.SkillDetail{
		SkillSummary: e.toSummary(),
		Versions:     []reg.SkillVersion{{ContentHash: e.hash, ScanTier: reg.ScanSafe}},
		EvalCells:    []reg.EvalSummaryCell{},
	}
	writeJSON(w, http.StatusOK, detail)
}

func (s *registryServer) handleResolve(w http.ResponseWriter, slug string) {
	e, ok := s.index[slug]
	if !ok {
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "not found"})
		return
	}
	writeJSON(w, http.StatusOK, reg.ResolveResponse{
		Slug:          e.Slug,
		Version:       e.hash,
		SourceType:    "github",
		SourceURL:     e.SourceURL,
		ContentHash:   e.hash,
		ScanTier:      reg.ScanSafe,
		InlineContent: e.Content,
		Tombstoned:    false,
	})
}

func (e *catalogEntry) toSummary() reg.SkillSummary {
	return reg.SkillSummary{
		Slug:          e.Slug,
		DisplayName:   e.DisplayName,
		Category:      e.Category,
		Stacks:        e.Stacks,
		SourceType:    "github",
		SourceURL:     e.SourceURL,
		LatestVersion: e.hash,
		ScanTier:      reg.ScanSafe,
		RatingCount:   0,
		Summary:       e.Summary,
	}
}

// matchesQuery returns true when EVERY whitespace-separated word of the query is a
// substring of the skill's searchable text, or when the whole query with spaces
// removed is a contiguous substring. Requiring all words (rather than any) keeps
// "front end" matching frontend skills without also matching "backend"/"dependency"
// on the stray word "end".
func matchesQuery(e *catalogEntry, query string) bool {
	hay := strings.ToLower(strings.Join([]string{
		e.Slug, e.DisplayName, e.Summary, e.Category, strings.Join(e.Stacks, " "),
	}, " "))
	if collapsed := stripSpace(query); collapsed != "" && strings.Contains(stripSpace(hay), collapsed) {
		return true
	}
	words := strings.Fields(query)
	if len(words) == 0 {
		return false
	}
	for _, word := range words {
		if !strings.Contains(hay, word) {
			return false
		}
	}
	return true
}

func stripSpace(s string) string {
	return strings.NewReplacer(" ", "", "-", "", "_", "").Replace(s)
}

func containsFold(list []string, target string) bool {
	for _, s := range list {
		if strings.EqualFold(s, target) {
			return true
		}
	}
	return false
}
