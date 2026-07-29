#!/bin/sh
# Reproduce the Acceptance Criteria queries from
# docs/agent-initiated-skill-discovery-spec.md against local Claude Code
# transcripts. Read-only, no network, no telemetry — the whole measurement
# story for the trigger rewrite is "count what is already on this disk".
#
# Run it once BEFORE the rewrite reaches an install and once after. Without the
# before-run there is nothing to compare against and A1–A4 are unmeasurable
# after the fact, so the baseline this repo measured on 2026-07-29 is printed
# below for reference.
#
#   sh scripts/measure-skill-triggers.sh [transcript-dir]
#
# Default transcript-dir is ~/.claude/projects.

set -eu

dir="${1:-$HOME/.claude/projects}"
if [ ! -d "$dir" ]; then
  echo "no transcript directory at $dir" >&2
  exit 1
fi

# Sessions, not events: a skill that fires twice in one session is still one
# session's worth of interruption, and the spec's rates are per session.
#
# A full scan of a few thousand transcripts takes minutes. That is fine — this
# runs twice per measurement window, not per commit.
count_sessions() {
  grep -rlF --include='*.jsonl' -e "$1" "$dir" 2>/dev/null | wc -l | tr -d ' '
}

count_events() {
  grep -rhoF --include='*.jsonl' -e "$1" "$dir" 2>/dev/null | wc -l | tr -d ' '
}

rate() {
  # $1 invoked, $2 present. Guard the divide: a skill nobody has is 0%, not an
  # error.
  if [ "$2" -eq 0 ]; then
    echo "n/a"
  else
    awk -v a="$1" -v b="$2" 'BEGIN { printf "%.2f%%", (a / b) * 100 }'
  fi
}

total=$(find "$dir" -name '*.jsonl' | wc -l | tr -d ' ')
echo "transcripts: $total  ($dir)"
echo

# "present" counts the available-skills listing line the harness injects
# ("- <name>: <description>"). It is one way of counting exposure, not the only
# one — what matters is that before and after runs use the same one.
printf '%-32s %8s %8s %8s\n' 'skill' 'present' 'invoked' 'rate'
for skill in skillrank find-skills playwright-cli \
  superpowers:brainstorming superpowers:systematic-debugging; do
  present=$(count_sessions "- $skill: ")
  invoked=$(count_sessions "\"name\":\"Skill\",\"input\":{\"skill\":\"$skill\"")
  printf '%-32s %8s %8s %8s\n' "$skill" "$present" "$invoked" "$(rate "$invoked" "$present")"
done

echo
mcp_present=$(count_sessions 'mcp__skillrank__skill_search')
mcp_sessions=$(count_sessions '"name":"mcp__skillrank__skill_')
mcp_calls=$(count_events '"name":"mcp__skillrank__skill_')
echo "skillrank MCP tools in context (sessions): $mcp_present"
echo "sessions with an mcp__skillrank__* call:   $mcp_sessions"
echo "total mcp__skillrank__* calls:             $mcp_calls"

cat <<'BASELINE'

Baseline recorded 2026-07-29, pre-rewrite, 3,285 transcripts on the spec
author's machine:

  skillrank                 present 2,569  invoked     0   0.00%
  find-skills               present 3,022  invoked     1   0.03%
  playwright-cli            present 2,511  invoked    38   1.51%
  brainstorming             present 3,048  invoked    20   0.66%
  systematic-debugging      present 3,047  invoked    16   0.52%
  skillrank MCP: 2,219 sessions in context, 1 session calling, 3 calls total

Acceptance criteria (spec §Acceptance Criteria):
  A1 fires at all      >=1 Skill invocation or >=5 MCP calls, unprompted
  A2 right band        0.5%-1.5% of sessions
  A3 nagware ceiling   <3% of sessions -- hard gate, revert if tripped
  A4 precision         >=50% of agent-initiated installs still in
                       skill-registry-lock.json 7 days later
BASELINE
