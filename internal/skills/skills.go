// Package skills discovers the skill surface directories in a repo. It mirrors
// the conventional locations agents read from, so skillrank installs land where
// Claude Code, Codex, and others already look.
package skills

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// SupportedDirectories are the skill-surface locations, in priority order.
var SupportedDirectories = []string{
	".agents/skills",
	".claude/skills",
	".codex/skills",
	".agent/skills",
}

// Surface is a discovered skill directory.
type Surface struct {
	RelativePath string `json:"relativePath"`
	AbsolutePath string `json:"absolutePath"`
}

// Skill is a discovered on-disk skill.
type Skill struct {
	Name                string `json:"name"`
	RelativePath        string `json:"relativePath"`
	AbsolutePath        string `json:"absolutePath"`
	SurfaceRelativePath string `json:"surfaceRelativePath"`
}

// DiscoveryResult is the outcome of scanning a repo.
type DiscoveryResult struct {
	SupportedDirectories []string `json:"supportedDirectories"`
	Surface              *Surface `json:"surface,omitempty"`
	Skills               []Skill  `json:"skills"`
}

// Discover scans repoRoot for skill surfaces and the skills within them.
func Discover(repoRoot string) (DiscoveryResult, error) {
	root := strings.TrimSpace(repoRoot)
	if root == "" {
		return DiscoveryResult{}, fmt.Errorf("repo root is required")
	}
	result := DiscoveryResult{SupportedDirectories: append([]string(nil), SupportedDirectories...)}
	for _, relative := range SupportedDirectories {
		absolute := filepath.Join(root, filepath.FromSlash(relative))
		info, err := os.Stat(absolute)
		if err == nil && info.IsDir() {
			surface := Surface{RelativePath: relative, AbsolutePath: absolute}
			if result.Surface == nil {
				result.Surface = &surface
			}
			found, err := listSkillsInSurface(surface)
			if err != nil {
				return DiscoveryResult{}, err
			}
			result.Skills = append(result.Skills, found...)
			continue
		}
		if err != nil && !os.IsNotExist(err) {
			return DiscoveryResult{}, fmt.Errorf("inspect skill directory %s: %w", relative, err)
		}
	}
	return result, nil
}

func listSkillsInSurface(surface Surface) ([]Skill, error) {
	entries, err := os.ReadDir(surface.AbsolutePath)
	if err != nil {
		return nil, fmt.Errorf("read skill directory %s: %w", surface.RelativePath, err)
	}
	var result []Skill
	for _, entry := range entries {
		if strings.HasPrefix(entry.Name(), ".") {
			continue
		}
		var skillPath string
		if entry.IsDir() {
			candidate := filepath.Join(surface.AbsolutePath, entry.Name(), "SKILL.md")
			if _, err := os.Stat(candidate); err == nil {
				skillPath = candidate
			}
		} else if strings.EqualFold(entry.Name(), "SKILL.md") || strings.HasSuffix(strings.ToLower(entry.Name()), ".md") {
			skillPath = filepath.Join(surface.AbsolutePath, entry.Name())
		}
		if skillPath == "" {
			continue
		}
		relative, err := filepath.Rel(surface.AbsolutePath, skillPath)
		if err != nil {
			relative = skillPath
		}
		repoRelative := filepath.ToSlash(filepath.Join(surface.RelativePath, relative))
		name := ParseManifestName(readFileString(skillPath))
		if strings.TrimSpace(name) == "" {
			name = fallbackSkillName(skillPath)
		}
		result = append(result, Skill{
			Name:                name,
			RelativePath:        repoRelative,
			AbsolutePath:        skillPath,
			SurfaceRelativePath: surface.RelativePath,
		})
	}
	return result, nil
}

// ParseManifestName reads the `name:` field from SKILL.md YAML frontmatter.
func ParseManifestName(content string) string {
	content = strings.TrimPrefix(content, "\ufeff")
	lines := strings.Split(content, "\n")
	if len(lines) == 0 || strings.TrimSpace(lines[0]) != "---" {
		return ""
	}
	for _, line := range lines[1:] {
		trimmed := strings.TrimSpace(line)
		if trimmed == "---" {
			break
		}
		name, ok := strings.CutPrefix(trimmed, "name:")
		if !ok {
			continue
		}
		return strings.Trim(strings.TrimSpace(name), `"'`)
	}
	return ""
}

func readFileString(path string) string {
	data, err := os.ReadFile(path)
	if err != nil {
		return ""
	}
	return string(data)
}

func fallbackSkillName(path string) string {
	base := filepath.Base(path)
	if strings.EqualFold(base, "SKILL.md") {
		parent := filepath.Base(filepath.Dir(path))
		if parent != "." && parent != string(filepath.Separator) {
			return parent
		}
	}
	return strings.TrimSuffix(base, filepath.Ext(base))
}
