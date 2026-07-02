package runner

import "testing"

func TestParseClaudeUsage(t *testing.T) {
	stdout := []byte(`{"type":"result","subtype":"success","total_cost_usd":0.1234,"num_turns":5,"duration_ms":8200,"is_error":false,"usage":{"input_tokens":1200,"output_tokens":800,"cache_read_input_tokens":5000,"cache_creation_input_tokens":300}}`)
	out, err := parseClaudeUsage(stdout)
	if err != nil {
		t.Fatal(err)
	}
	if out.InputTokens != 1200 || out.OutputTokens != 800 {
		t.Errorf("token mismatch: %+v", out)
	}
	if out.CacheRead != 5000 || out.CacheWrite != 300 {
		t.Errorf("cache token mismatch: %+v", out)
	}
	if out.CostUSD == nil || *out.CostUSD != 0.1234 {
		t.Errorf("cost mismatch: %+v", out.CostUSD)
	}
	if out.Turns != 5 {
		t.Errorf("turns mismatch: %d", out.Turns)
	}
}

func TestParseCodexUsageAccumulatesTurns(t *testing.T) {
	stdout := []byte(`{"type":"thread.started"}
{"type":"turn.completed","usage":{"input_tokens":500,"cached_input_tokens":100,"output_tokens":200,"reasoning_output_tokens":50}}
{"type":"turn.completed","usage":{"input_tokens":300,"cached_input_tokens":80,"output_tokens":150,"reasoning_output_tokens":25}}`)
	out, err := parseCodexUsage(stdout)
	if err != nil {
		t.Fatal(err)
	}
	if out.InputTokens != 800 {
		t.Errorf("expected 800 input tokens, got %d", out.InputTokens)
	}
	// output = (200+50)+(150+25) = 425
	if out.OutputTokens != 425 {
		t.Errorf("expected 425 output tokens (incl reasoning), got %d", out.OutputTokens)
	}
	if out.CacheRead != 180 {
		t.Errorf("expected 180 cached, got %d", out.CacheRead)
	}
	if out.Turns != 2 {
		t.Errorf("expected 2 turns, got %d", out.Turns)
	}
}

func TestParseCodexUsageNoTurnsIsError(t *testing.T) {
	if _, err := parseCodexUsage([]byte(`{"type":"thread.started"}`)); err == nil {
		t.Fatal("expected error when no turn.completed events present")
	}
}

func TestVersionBand(t *testing.T) {
	cases := map[string]string{
		"2.1.174":  "2.1",
		"v2.1.176": "2.1",
		"1.0":      "1.0",
		"3":        "3",
	}
	for in, want := range cases {
		if got := VersionBand(in); got != want {
			t.Errorf("VersionBand(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestBuildPromptForcedModeVsControl(t *testing.T) {
	control := buildPrompt(RunSpec{Instruction: "fix the bug", SkillInstalled: false})
	if control != "fix the bug" {
		t.Errorf("control prompt should be the bare instruction, got %q", control)
	}
	treatment := buildPrompt(RunSpec{Instruction: "fix the bug", SkillInstalled: true, SkillSlug: "owner/skill"})
	if treatment == control {
		t.Error("treatment prompt should differ from control (forced skill invocation)")
	}
	if !contains(treatment, ".claude/skills/owner/skill/SKILL.md") {
		t.Errorf("treatment prompt should force-invoke the installed skill, got %q", treatment)
	}
}

func contains(haystack, needle string) bool {
	return len(haystack) >= len(needle) && (indexOf(haystack, needle) >= 0)
}

func indexOf(s, sub string) int {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return i
		}
	}
	return -1
}
