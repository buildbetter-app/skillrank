package registry

import (
	"os"
	"path/filepath"
	"strings"
)

// DetectedStack is a repo signal used by `recommend` to filter the registry.
type DetectedStack struct {
	Stacks   []string `json:"stacks"`
	Evidence []string `json:"evidence"`
}

// stackProbe maps a marker file (and optional content substring) to a stack tag.
type stackProbe struct {
	file      string
	needle    string // optional substring that must appear in the file
	stack     string
	evidence  string
}

var stackProbes = []stackProbe{
	{file: "next.config.js", stack: "nextjs", evidence: "next.config.js"},
	{file: "next.config.mjs", stack: "nextjs", evidence: "next.config.mjs"},
	{file: "next.config.ts", stack: "nextjs", evidence: "next.config.ts"},
	{file: "components.json", stack: "shadcn", evidence: "components.json (shadcn/ui)"},
	{file: "package.json", needle: "\"next\"", stack: "nextjs", evidence: "next dependency in package.json"},
	{file: "package.json", needle: "\"react\"", stack: "react", evidence: "react dependency in package.json"},
	{file: "package.json", needle: "\"@playwright/test\"", stack: "playwright", evidence: "@playwright/test in package.json"},
	{file: "package.json", needle: "\"express\"", stack: "node-api", evidence: "express in package.json"},
	{file: "package.json", needle: "\"hono\"", stack: "node-api", evidence: "hono in package.json"},
	{file: "go.mod", stack: "go", evidence: "go.mod"},
	{file: "pyproject.toml", needle: "fastapi", stack: "fastapi", evidence: "fastapi in pyproject.toml"},
	{file: "requirements.txt", needle: "fastapi", stack: "fastapi", evidence: "fastapi in requirements.txt"},
	{file: "manage.py", stack: "django", evidence: "manage.py (Django)"},
	{file: "pyproject.toml", needle: "django", stack: "django", evidence: "django in pyproject.toml"},
	{file: "Gemfile", needle: "rails", stack: "rails", evidence: "rails in Gemfile"},
	{file: "pom.xml", stack: "java", evidence: "pom.xml"},
	{file: "build.gradle", stack: "java", evidence: "build.gradle"},
}

// DetectStack inspects marker files at the repo root to infer the stack(s).
func DetectStack(repoRoot string) DetectedStack {
	result := DetectedStack{}
	stackSet := map[string]bool{}
	for _, probe := range stackProbes {
		path := filepath.Join(repoRoot, probe.file)
		data, err := os.ReadFile(path)
		if err != nil {
			continue
		}
		if probe.needle != "" && !strings.Contains(string(data), probe.needle) {
			continue
		}
		if !stackSet[probe.stack] {
			stackSet[probe.stack] = true
			result.Stacks = append(result.Stacks, probe.stack)
		}
		result.Evidence = append(result.Evidence, probe.evidence)
	}
	return result
}
