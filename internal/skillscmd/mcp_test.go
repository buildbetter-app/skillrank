package skillscmd

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/buildbetter/skillrank/internal/command"
	reg "github.com/buildbetter/skillrank/internal/registry"
)

func newTestServer(t *testing.T, client reg.Client) *mcpServer {
	t.Helper()
	return &mcpServer{client: client, version: "test", stderr: &bytes.Buffer{}}
}

func decodeResp(t *testing.T, out *bytes.Buffer) rpcResponse {
	t.Helper()
	var resp rpcResponse
	if err := json.Unmarshal(bytes.TrimSpace(out.Bytes()), &resp); err != nil {
		t.Fatalf("bad rpc response %q: %v", out.String(), err)
	}
	return resp
}

func TestMCPInitializeEchoesProtocolVersion(t *testing.T) {
	s := newTestServer(t, reg.Client{})
	var out bytes.Buffer
	s.handleLine([]byte(`{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{}}}`), &out)
	resp := decodeResp(t, &out)
	result := resp.Result.(map[string]any)
	if result["protocolVersion"] != "2025-11-25" {
		t.Fatalf("expected echoed protocol version, got %v", result["protocolVersion"])
	}
	if _, ok := result["capabilities"].(map[string]any)["tools"]; !ok {
		t.Fatal("expected tools capability advertised")
	}
	si := result["serverInfo"].(map[string]any)
	if si["name"] != "skillrank" {
		t.Fatalf("unexpected serverInfo: %v", si)
	}
}

func TestMCPToolsListExposesSkillTools(t *testing.T) {
	s := newTestServer(t, reg.Client{})
	var out bytes.Buffer
	s.handleLine([]byte(`{"jsonrpc":"2.0","id":2,"method":"tools/list"}`), &out)
	resp := decodeResp(t, &out)
	tools := resp.Result.(map[string]any)["tools"].([]any)
	names := map[string]bool{}
	for _, tl := range tools {
		names[tl.(map[string]any)["name"].(string)] = true
	}
	for _, want := range []string{"skill_search", "skill_show", "skill_recommend", "skill_install", "skill_list"} {
		if !names[want] {
			t.Errorf("tools/list missing %q (got %v)", want, names)
		}
	}
}

func TestMCPNotificationProducesNoResponse(t *testing.T) {
	s := newTestServer(t, reg.Client{})
	var out bytes.Buffer
	s.handleLine([]byte(`{"jsonrpc":"2.0","method":"notifications/initialized"}`), &out)
	if strings.TrimSpace(out.String()) != "" {
		t.Fatalf("notification must not produce a response, got %q", out.String())
	}
}

func TestMCPToolCallSearchHitsRegistry(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/skills") {
			_ = json.NewEncoder(w).Encode(reg.SearchResponse{
				Items: []reg.SkillSummary{{Slug: "demo/pw", ScanTier: reg.ScanSafe, Summary: "Playwright tests", Stacks: []string{"playwright"}}},
				Total: 1,
			})
			return
		}
		http.NotFound(w, r)
	}))
	defer server.Close()

	client, _ := reg.NewClient(server.URL, nil)
	s := newTestServer(t, client)
	var out bytes.Buffer
	s.handleLine([]byte(`{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"skill_search","arguments":{"query":"playwright"}}}`), &out)
	resp := decodeResp(t, &out)
	result := resp.Result.(map[string]any)
	if result["isError"] == true {
		t.Fatalf("tool reported error: %v", result)
	}
	text := result["content"].([]any)[0].(map[string]any)["text"].(string)
	if !strings.Contains(text, "demo/pw") {
		t.Fatalf("expected search result in tool output, got: %s", text)
	}
}

func TestMCPRunReadsMultipleMessages(t *testing.T) {
	// Two newline-delimited messages in, two responses (initialize + tools/list).
	in := strings.NewReader(
		`{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}` + "\n" +
			`{"jsonrpc":"2.0","id":2,"method":"tools/list"}` + "\n")
	var out bytes.Buffer
	ctx := command.Context{Stdin: in, Stdout: &out, Stderr: &bytes.Buffer{}, Version: "test", HTTPClient: http.DefaultClient}
	if code := runMCP(nil, ctx); code != 0 {
		t.Fatalf("runMCP exit = %d", code)
	}
	lines := strings.Split(strings.TrimSpace(out.String()), "\n")
	if len(lines) != 2 {
		t.Fatalf("expected 2 responses, got %d: %q", len(lines), out.String())
	}
}
