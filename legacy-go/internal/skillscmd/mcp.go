package skillscmd

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"strings"

	"github.com/buildbetter/skillrank/internal/command"
	reg "github.com/buildbetter/skillrank/internal/registry"
)

// runMCP runs skillrank as a Model Context Protocol (MCP) stdio server. Once
// registered with Claude Code or Codex (see `skillrank setup`), the agent gets
// first-class tools — skill_search, skill_show, skill_recommend, skill_install,
// skill_list — so "find me a skill for playwright" just works, in the agent's own
// tool vocabulary, without the user knowing any command.
//
// Transport: newline-delimited JSON-RPC 2.0 over stdio. Protocol JSON goes to
// stdout ONLY; everything else must go to stderr.
func runMCP(args []string, ctx command.Context) int {
	flags := command.ParseFlags(args)
	client, err := reg.NewClient(flags.Values["api-base-url"], ctx.HTTPClient)
	if err != nil {
		fmt.Fprintf(ctx.Stderr, "skillrank mcp: %s\n", err)
		return 1
	}
	srv := &mcpServer{client: client, version: ctx.Version, stderr: ctx.Stderr}

	reader := bufio.NewReader(ctx.Stdin)
	writer := ctx.Stdout
	for {
		line, err := reader.ReadBytes('\n')
		if len(strings.TrimSpace(string(line))) > 0 {
			srv.handleLine(line, writer)
		}
		if err != nil {
			if err == io.EOF {
				return 0
			}
			fmt.Fprintf(ctx.Stderr, "skillrank mcp: read error: %s\n", err)
			return 1
		}
	}
}

type mcpServer struct {
	client  reg.Client
	version string
	stderr  io.Writer
}

type rpcRequest struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id,omitempty"`
	Method  string          `json:"method"`
	Params  json.RawMessage `json:"params,omitempty"`
}

type rpcResponse struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id,omitempty"`
	Result  any             `json:"result,omitempty"`
	Error   *rpcError       `json:"error,omitempty"`
}

type rpcError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
}

func (s *mcpServer) handleLine(line []byte, w io.Writer) {
	var req rpcRequest
	if err := json.Unmarshal(line, &req); err != nil {
		return // ignore unparseable input
	}
	// Notifications have no id and expect no response.
	isNotification := len(req.ID) == 0 || string(req.ID) == "null"

	switch req.Method {
	case "initialize":
		s.reply(w, req.ID, s.initializeResult(req.Params))
	case "notifications/initialized", "initialized":
		// no response
	case "ping":
		s.reply(w, req.ID, map[string]any{})
	case "tools/list":
		s.reply(w, req.ID, map[string]any{"tools": toolDefinitions()})
	case "tools/call":
		s.reply(w, req.ID, s.callTool(req.Params))
	default:
		if !isNotification {
			s.replyError(w, req.ID, -32601, "method not found: "+req.Method)
		}
	}
}

func (s *mcpServer) initializeResult(params json.RawMessage) map[string]any {
	// Echo the client's requested protocol version when present (servers should
	// agree to the client's version or offer their own supported one).
	protocolVersion := "2025-06-18"
	var p struct {
		ProtocolVersion string `json:"protocolVersion"`
	}
	if len(params) > 0 && json.Unmarshal(params, &p) == nil && strings.TrimSpace(p.ProtocolVersion) != "" {
		protocolVersion = p.ProtocolVersion
	}
	return map[string]any{
		"protocolVersion": protocolVersion,
		"capabilities":    map[string]any{"tools": map[string]any{}},
		"serverInfo":      map[string]any{"name": "skillrank", "version": s.version},
		"instructions": "skillrank finds, installs, and evaluates agent skills. Use skill_search or " +
			"skill_recommend to find skills, skill_show to inspect one, and skill_install to add it " +
			"to this repo (it becomes available to the agent automatically).",
	}
}

func (s *mcpServer) reply(w io.Writer, id json.RawMessage, result any) {
	if len(id) == 0 {
		return // notification: no reply
	}
	s.write(w, rpcResponse{JSONRPC: "2.0", ID: id, Result: result})
}

func (s *mcpServer) replyError(w io.Writer, id json.RawMessage, code int, msg string) {
	s.write(w, rpcResponse{JSONRPC: "2.0", ID: id, Error: &rpcError{Code: code, Message: msg}})
}

func (s *mcpServer) write(w io.Writer, resp rpcResponse) {
	buf, err := json.Marshal(resp)
	if err != nil {
		return
	}
	buf = append(buf, '\n')
	_, _ = w.Write(buf)
}

// toolText wraps a tool result as MCP content. isError marks a failed call.
func toolText(text string, isError bool) map[string]any {
	return map[string]any{
		"content": []map[string]any{{"type": "text", "text": text}},
		"isError": isError,
	}
}

func (s *mcpServer) callTool(params json.RawMessage) map[string]any {
	var call struct {
		Name      string          `json:"name"`
		Arguments json.RawMessage `json:"arguments"`
	}
	if err := json.Unmarshal(params, &call); err != nil {
		return toolText("invalid tool call params", true)
	}
	switch call.Name {
	case "skill_search":
		return s.toolSearch(call.Arguments)
	case "skill_show":
		return s.toolShow(call.Arguments)
	case "skill_recommend":
		return s.toolRecommend(call.Arguments)
	case "skill_install":
		return s.toolInstall(call.Arguments)
	case "skill_list":
		return s.toolList(call.Arguments)
	default:
		return toolText("unknown tool: "+call.Name, true)
	}
}

func (s *mcpServer) toolSearch(argsRaw json.RawMessage) map[string]any {
	var a struct {
		Query    string `json:"query"`
		Stack    string `json:"stack"`
		Agent    string `json:"agent"`
		Category string `json:"category"`
		Limit    int    `json:"limit"`
	}
	_ = json.Unmarshal(argsRaw, &a)
	limit := a.Limit
	if limit <= 0 {
		limit = 15
	}
	resp, err := s.client.Search(reg.SearchOptions{Query: a.Query, Stack: a.Stack, Agent: a.Agent, Category: a.Category, Limit: limit})
	if err != nil {
		return toolText("search failed: "+err.Error(), true)
	}
	if len(resp.Items) == 0 {
		return toolText("No skills matched \""+a.Query+"\".", false)
	}
	var b strings.Builder
	fmt.Fprintf(&b, "%d skill(s) for %q:\n", len(resp.Items), a.Query)
	for _, item := range resp.Items {
		fmt.Fprintf(&b, "- %s (scan: %s)", item.Slug, item.ScanTier)
		if len(item.Stacks) > 0 {
			fmt.Fprintf(&b, " [%s]", strings.Join(item.Stacks, ","))
		}
		if item.Summary != "" {
			fmt.Fprintf(&b, " — %s", item.Summary)
		}
		b.WriteString("\n")
	}
	b.WriteString("\nInstall one with the skill_install tool (ref = the slug).")
	return toolText(b.String(), false)
}

func (s *mcpServer) toolShow(argsRaw json.RawMessage) map[string]any {
	var a struct {
		Slug string `json:"slug"`
	}
	_ = json.Unmarshal(argsRaw, &a)
	if strings.TrimSpace(a.Slug) == "" {
		return toolText("slug is required", true)
	}
	detail, err := s.client.Show(a.Slug)
	if err != nil {
		return toolText("show failed: "+err.Error(), true)
	}
	buf, _ := json.MarshalIndent(detail, "", "  ")
	return toolText(string(buf), false)
}

func (s *mcpServer) toolRecommend(argsRaw json.RawMessage) map[string]any {
	var a struct {
		Cwd string `json:"cwd"`
	}
	_ = json.Unmarshal(argsRaw, &a)
	repoRoot := reg.RepoRoot(a.Cwd)
	detected := reg.DetectStack(repoRoot)
	if len(detected.Stacks) == 0 {
		return toolText("Could not detect a stack in this repo. Use skill_search with a query instead.", false)
	}
	var b strings.Builder
	fmt.Fprintf(&b, "Detected stack: %s\n", strings.Join(detected.Stacks, ", "))
	seen := map[string]bool{}
	found := 0
	for _, stack := range detected.Stacks {
		resp, err := s.client.Search(reg.SearchOptions{Stack: stack, Sort: "signals", Limit: 5})
		if err != nil {
			continue
		}
		for _, item := range resp.Items {
			if seen[item.Slug] {
				continue
			}
			seen[item.Slug] = true
			found++
			fmt.Fprintf(&b, "- %s (scan: %s) — %s\n", item.Slug, item.ScanTier, item.Summary)
		}
	}
	if found == 0 {
		b.WriteString("No matching skills in the registry yet.\n")
	}
	return toolText(b.String(), false)
}

func (s *mcpServer) toolInstall(argsRaw json.RawMessage) map[string]any {
	var a struct {
		Ref     string `json:"ref"`
		Surface string `json:"surface"`
		Cwd     string `json:"cwd"`
		Yes     bool   `json:"yes"`
	}
	_ = json.Unmarshal(argsRaw, &a)
	if strings.TrimSpace(a.Ref) == "" {
		return toolText("ref (skill slug) is required", true)
	}
	repoRoot := reg.RepoRoot(a.Cwd)
	// Pre-flight: refuse an unsafe scan tier unless the caller explicitly confirms.
	resolved, err := s.client.Resolve(a.Ref)
	if err != nil {
		return toolText("resolve failed: "+err.Error(), true)
	}
	if !reg.SafeScanTier(resolved.ScanTier) && !a.Yes {
		return toolText(fmt.Sprintf(
			"%s has scan tier %q (not verified safe). Ask the user to confirm, then call skill_install again with yes=true.",
			resolved.Slug, resolved.ScanTier), true)
	}
	result, err := s.client.Install(reg.InstallOptions{Ref: a.Ref, RepoRoot: repoRoot, SurfaceOverride: a.Surface})
	if err != nil {
		return toolText("install failed: "+err.Error(), true)
	}
	if result.AlreadyExact {
		return toolText(fmt.Sprintf("%s is already installed at %s (up to date).", result.Slug, result.SkillPath), false)
	}
	return toolText(fmt.Sprintf(
		"Installed %s → %s (scan: %s). It is now available to the agent in this repo automatically.",
		result.Slug, result.SkillPath, result.ScanTier), false)
}

func (s *mcpServer) toolList(argsRaw json.RawMessage) map[string]any {
	var a struct {
		Cwd string `json:"cwd"`
	}
	_ = json.Unmarshal(argsRaw, &a)
	rows, err := reg.ListInstalled(reg.RepoRoot(a.Cwd))
	if err != nil {
		return toolText("list failed: "+err.Error(), true)
	}
	if len(rows) == 0 {
		return toolText("No registry-installed skills in this repo.", false)
	}
	var b strings.Builder
	for _, r := range rows {
		fmt.Fprintf(&b, "- %s [%s] %s\n", r.Slug, r.State, r.SkillPath)
	}
	return toolText(b.String(), false)
}

// toolDefinitions is the tools/list payload: the agent's new vocabulary.
func toolDefinitions() []map[string]any {
	strProp := func(desc string) map[string]any { return map[string]any{"type": "string", "description": desc} }
	return []map[string]any{
		{
			"name":        "skill_search",
			"description": "Search the public skill registry for agent skills. Use when the user asks to find a skill for something (e.g. 'find me a skill for playwright').",
			"inputSchema": map[string]any{
				"type":     "object",
				"required": []string{"query"},
				"properties": map[string]any{
					"query":    strProp("What to search for, e.g. 'playwright' or 'react performance'."),
					"stack":    strProp("Optional stack filter, e.g. nextjs, fastapi, go, playwright."),
					"agent":    strProp("Optional agent filter, e.g. claude, codex."),
					"category": strProp("Optional category filter."),
					"limit":    map[string]any{"type": "integer", "description": "Max results (default 15)."},
				},
			},
		},
		{
			"name":        "skill_recommend",
			"description": "Recommend skills for the current repository by detecting its stack. Use when the user asks 'what skills should I use here'.",
			"inputSchema": map[string]any{
				"type": "object",
				"properties": map[string]any{
					"cwd": strProp("Repo directory to inspect (default: current working directory)."),
				},
			},
		},
		{
			"name":        "skill_show",
			"description": "Show a skill's details, security scan tier, and eval results by trust tier. Use to evaluate whether a skill is worth installing.",
			"inputSchema": map[string]any{
				"type":       "object",
				"required":   []string{"slug"},
				"properties": map[string]any{"slug": strProp("The skill slug, e.g. owner/skill.")},
			},
		},
		{
			"name":        "skill_install",
			"description": "Install a skill into this repository (hash-verified). It becomes available to the agent automatically. If the scan tier is unsafe, the tool asks for confirmation; re-call with yes=true after the user agrees.",
			"inputSchema": map[string]any{
				"type":     "object",
				"required": []string{"ref"},
				"properties": map[string]any{
					"ref":     strProp("Skill slug (optionally slug@version)."),
					"surface": strProp("Optional skill surface dir, e.g. .claude/skills or .agents/skills."),
					"cwd":     strProp("Repo directory (default: current working directory)."),
					"yes":     map[string]any{"type": "boolean", "description": "Confirm install despite an unsafe scan tier."},
				},
			},
		},
		{
			"name":        "skill_list",
			"description": "List skills installed in this repo via skillrank, including drift (modified/removed).",
			"inputSchema": map[string]any{
				"type": "object",
				"properties": map[string]any{
					"cwd": strProp("Repo directory (default: current working directory)."),
				},
			},
		},
	}
}
