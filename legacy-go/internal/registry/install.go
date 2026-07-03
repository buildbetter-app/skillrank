package registry

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// InstallResult reports what an install did.
type InstallResult struct {
	Slug         string   `json:"slug"`
	Version      string   `json:"version"`
	SkillPath    string   `json:"skillPath"`
	Surface      string   `json:"surface"`
	ScanTier     ScanTier `json:"scanTier"`
	ContentHash  string   `json:"contentHash"`
	AlreadyExact bool     `json:"alreadyExact"`
}

// InstallOptions parameterize Install.
type InstallOptions struct {
	Ref            string
	RepoRoot       string
	SurfaceOverride string
	// NowRFC3339 is injected for deterministic tests; empty uses time.Now.
	NowRFC3339 string
}

// SafeScanTier reports whether a tier is safe to install without an extra
// confirmation prompt.
func SafeScanTier(tier ScanTier) bool {
	switch tier {
	case ScanSafe, ScanLow:
		return true
	default:
		return false
	}
}

// Install resolves, verifies, and writes a skill into the repo surface, updating
// the lockfile. It never executes skill content. The caller is responsible for
// any confirmation prompt when the scan tier is unsafe (see SafeScanTier).
func (c Client) Install(opts InstallOptions) (InstallResult, error) {
	resolved, err := c.Resolve(opts.Ref)
	if err != nil {
		if IsNotFound(err) {
			return InstallResult{}, fmt.Errorf("skill %q not found in the registry", opts.Ref)
		}
		return InstallResult{}, err
	}
	if resolved.Tombstoned {
		reason := resolved.TombstoneReason
		if reason == "" {
			reason = "removed upstream"
		}
		return InstallResult{}, fmt.Errorf("skill %q is unavailable: %s", resolved.Slug, reason)
	}

	content, err := c.fetchSkillContent(resolved)
	if err != nil {
		return InstallResult{}, err
	}

	// Verify content integrity before writing anything.
	got := ComputeContentHash(content)
	if !HashesEqual(got, resolved.ContentHash) {
		return InstallResult{}, fmt.Errorf(
			"content hash mismatch for %s: registry advertised %s but downloaded content hashes to %s; refusing to install",
			resolved.Slug, resolved.ContentHash, got)
	}

	surfaceRel, surfaceAbs, err := ResolveSurface(opts.RepoRoot, opts.SurfaceOverride)
	if err != nil {
		return InstallResult{}, err
	}
	skillDir := filepath.Join(surfaceAbs, resolved.Slug)
	skillFile := filepath.Join(skillDir, "SKILL.md")
	skillPathRel := filepath.ToSlash(filepath.Join(surfaceRel, resolved.Slug, "SKILL.md"))

	// Idempotence: if the exact content is already present, report and skip write.
	if existing, readErr := os.ReadFile(skillFile); readErr == nil {
		if HashesEqual(ComputeContentHash(string(existing)), resolved.ContentHash) {
			if err := c.recordLock(opts, resolved, skillPathRel, surfaceRel); err != nil {
				return InstallResult{}, err
			}
			return InstallResult{
				Slug: resolved.Slug, Version: resolved.Version, SkillPath: skillPathRel,
				Surface: surfaceRel, ScanTier: resolved.ScanTier, ContentHash: resolved.ContentHash,
				AlreadyExact: true,
			}, nil
		}
	}

	// Write atomically: temp file then rename, so a failed write leaves no
	// partial install.
	if err := os.MkdirAll(skillDir, 0o755); err != nil {
		return InstallResult{}, fmt.Errorf("create skill directory: %w", err)
	}
	tmp := skillFile + ".tmp"
	if err := os.WriteFile(tmp, []byte(content), 0o644); err != nil {
		return InstallResult{}, fmt.Errorf("write skill content: %w", err)
	}
	if err := os.Rename(tmp, skillFile); err != nil {
		_ = os.Remove(tmp)
		return InstallResult{}, fmt.Errorf("finalize skill install: %w", err)
	}

	if err := c.recordLock(opts, resolved, skillPathRel, surfaceRel); err != nil {
		return InstallResult{}, err
	}
	return InstallResult{
		Slug: resolved.Slug, Version: resolved.Version, SkillPath: skillPathRel,
		Surface: surfaceRel, ScanTier: resolved.ScanTier, ContentHash: resolved.ContentHash,
	}, nil
}

func (c Client) fetchSkillContent(resolved ResolveResponse) (string, error) {
	if strings.TrimSpace(resolved.InlineContent) != "" {
		return resolved.InlineContent, nil
	}
	if strings.TrimSpace(resolved.RawContentURL) != "" {
		return c.FetchRawContent(resolved.RawContentURL)
	}
	return "", fmt.Errorf("registry did not provide installable content for %s", resolved.Slug)
}

func (c Client) recordLock(opts InstallOptions, resolved ResolveResponse, skillPathRel, surfaceRel string) error {
	lf, err := LoadLockfile(opts.RepoRoot)
	if err != nil {
		return err
	}
	now := opts.NowRFC3339
	if now == "" {
		now = time.Now().UTC().Format(time.RFC3339)
	}
	ref := resolved.Slug
	if resolved.Version != "" {
		ref = resolved.Slug + "@" + resolved.Version
	}
	lf.Upsert(LockEntry{
		Slug:         resolved.Slug,
		RegistryRef:  ref,
		SourceType:   resolved.SourceType,
		Source:       resolved.SourceURL,
		SkillPath:    skillPathRel,
		Surface:      surfaceRel,
		ComputedHash: resolved.ContentHash,
		PinnedCommit: resolved.PinnedCommit,
		InstalledAt:  now,
	})
	return lf.Save()
}

// InstalledSkill is one row in `list`.
type InstalledSkill struct {
	Slug        string `json:"slug"`
	SkillPath   string `json:"skillPath"`
	Version     string `json:"version,omitempty"`
	State       string `json:"state"` // "ok" | "modified" | "untracked"
	Surface     string `json:"surface,omitempty"`
}

// ListInstalled reconciles the lockfile against on-disk surface content and
// reports drift.
func ListInstalled(repoRoot string) ([]InstalledSkill, error) {
	lf, err := LoadLockfile(repoRoot)
	if err != nil {
		return nil, err
	}
	seen := map[string]bool{}
	var rows []InstalledSkill
	for _, e := range lf.Skills {
		seen[e.SkillPath] = true
		state := "ok"
		abs := filepath.Join(repoRoot, filepath.FromSlash(e.SkillPath))
		content, readErr := os.ReadFile(abs)
		if readErr != nil {
			state = "removed upstream"
		} else if !HashesEqual(ComputeContentHash(string(content)), e.ComputedHash) {
			state = "modified"
		}
		rows = append(rows, InstalledSkill{
			Slug: e.Slug, SkillPath: e.SkillPath, Version: e.RegistryRef,
			State: state, Surface: e.Surface,
		})
	}
	return rows, nil
}

// Uninstall removes a skill's files and lockfile entry by slug.
func Uninstall(repoRoot, slug string) (string, error) {
	lf, err := LoadLockfile(repoRoot)
	if err != nil {
		return "", err
	}
	entry := lf.FindBySlug(slug)
	if entry == nil {
		return "", fmt.Errorf("skill %q is not installed (no lockfile entry)", slug)
	}
	skillPath := entry.SkillPath
	abs := filepath.Join(repoRoot, filepath.FromSlash(skillPath))
	// Remove the skill directory (parent of SKILL.md) when it looks like a
	// dedicated per-skill dir; otherwise remove just the file.
	dir := filepath.Dir(abs)
	if filepath.Base(dir) == slug {
		_ = os.RemoveAll(dir)
	} else {
		_ = os.Remove(abs)
	}
	lf.Remove(skillPath)
	if err := lf.Save(); err != nil {
		return "", err
	}
	return skillPath, nil
}
