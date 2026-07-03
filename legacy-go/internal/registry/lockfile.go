package registry

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// LockfileName is namespaced to avoid colliding with the `npx skills`
// ecosystem's own lockfile semantics (spec R4.2).
const LockfileName = "skill-registry-lock.json"

// LockEntry records one installed skill. Unknown fields present on disk are
// preserved on rewrite via Extra.
type LockEntry struct {
	Slug         string   `json:"slug"`
	RegistryRef  string   `json:"registryRef,omitempty"` // slug@version
	SourceType   string   `json:"sourceType,omitempty"`
	Source       string   `json:"source,omitempty"`
	SkillPath    string   `json:"skillPath"` // repo-relative install path
	Surface      string   `json:"surface,omitempty"`
	ComputedHash string   `json:"computedHash"`
	PinnedCommit string   `json:"pinnedCommit,omitempty"`
	InstalledAt  string   `json:"installedAt,omitempty"`
	Extra        RawExtra `json:"-"`
}

// Lockfile is the on-disk document. Foreign top-level keys are preserved.
type Lockfile struct {
	Version int          `json:"version"`
	Skills  []LockEntry  `json:"skills"`
	Extra   RawExtra     `json:"-"`
	path    string       `json:"-"`
}

// RawExtra preserves unrecognized JSON object fields across a read/write cycle.
type RawExtra map[string]json.RawMessage

// LockfilePath returns the lockfile path for a repo root.
func LockfilePath(repoRoot string) string {
	return filepath.Join(repoRoot, LockfileName)
}

// LoadLockfile reads the lockfile at repoRoot, returning an empty v1 lockfile if
// none exists. Unknown fields are retained.
func LoadLockfile(repoRoot string) (*Lockfile, error) {
	path := LockfilePath(repoRoot)
	lf := &Lockfile{Version: 1, path: path, Extra: RawExtra{}}
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return lf, nil
		}
		return nil, fmt.Errorf("read lockfile: %w", err)
	}
	// First pass: capture known + unknown top-level keys.
	var top map[string]json.RawMessage
	if err := json.Unmarshal(data, &top); err != nil {
		return nil, fmt.Errorf("parse lockfile %s: %w", path, err)
	}
	if raw, ok := top["version"]; ok {
		_ = json.Unmarshal(raw, &lf.Version)
		delete(top, "version")
	}
	if raw, ok := top["skills"]; ok {
		if err := lf.decodeSkills(raw); err != nil {
			return nil, err
		}
		delete(top, "skills")
	}
	lf.Extra = top
	if lf.Version == 0 {
		lf.Version = 1
	}
	return lf, nil
}

func (lf *Lockfile) decodeSkills(raw json.RawMessage) error {
	var rawEntries []map[string]json.RawMessage
	if err := json.Unmarshal(raw, &rawEntries); err != nil {
		return fmt.Errorf("parse lockfile skills: %w", err)
	}
	known := map[string]bool{
		"slug": true, "registryRef": true, "sourceType": true, "source": true,
		"skillPath": true, "surface": true, "computedHash": true,
		"pinnedCommit": true, "installedAt": true,
	}
	for _, rawEntry := range rawEntries {
		var entry LockEntry
		merged, _ := json.Marshal(rawEntry)
		if err := json.Unmarshal(merged, &entry); err != nil {
			return fmt.Errorf("parse lockfile entry: %w", err)
		}
		extra := RawExtra{}
		for k, v := range rawEntry {
			if !known[k] {
				extra[k] = v
			}
		}
		entry.Extra = extra
		lf.Skills = append(lf.Skills, entry)
	}
	return nil
}

// Upsert inserts or replaces an entry keyed by SkillPath.
func (lf *Lockfile) Upsert(entry LockEntry) {
	for i := range lf.Skills {
		if lf.Skills[i].SkillPath == entry.SkillPath {
			entry.Extra = mergeExtra(lf.Skills[i].Extra, entry.Extra)
			lf.Skills[i] = entry
			return
		}
	}
	lf.Skills = append(lf.Skills, entry)
}

// Remove deletes an entry by SkillPath, reporting whether one was removed.
func (lf *Lockfile) Remove(skillPath string) bool {
	for i := range lf.Skills {
		if lf.Skills[i].SkillPath == skillPath {
			lf.Skills = append(lf.Skills[:i], lf.Skills[i+1:]...)
			return true
		}
	}
	return false
}

// FindBySlug returns the first entry matching slug.
func (lf *Lockfile) FindBySlug(slug string) *LockEntry {
	for i := range lf.Skills {
		if strings.EqualFold(lf.Skills[i].Slug, slug) {
			return &lf.Skills[i]
		}
	}
	return nil
}

// Save writes the lockfile back, preserving foreign fields and using stable
// ordering. It only touches disk when there is content or an existing file.
func (lf *Lockfile) Save() error {
	// Sort entries by SkillPath for deterministic diffs.
	sort.SliceStable(lf.Skills, func(i, j int) bool {
		return lf.Skills[i].SkillPath < lf.Skills[j].SkillPath
	})
	out := map[string]json.RawMessage{}
	for k, v := range lf.Extra {
		out[k] = v
	}
	versionRaw, _ := json.Marshal(lf.Version)
	out["version"] = versionRaw
	skillsRaw, err := lf.encodeSkills()
	if err != nil {
		return err
	}
	out["skills"] = skillsRaw
	buf, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		return err
	}
	buf = append(buf, '\n')
	return os.WriteFile(lf.path, buf, 0o644)
}

func (lf *Lockfile) encodeSkills() (json.RawMessage, error) {
	entries := make([]map[string]json.RawMessage, 0, len(lf.Skills))
	for _, e := range lf.Skills {
		m := map[string]json.RawMessage{}
		for k, v := range e.Extra {
			m[k] = v
		}
		// Marshal known fields via a shadow struct (omitempty respected).
		knownBuf, err := json.Marshal(struct {
			Slug         string `json:"slug"`
			RegistryRef  string `json:"registryRef,omitempty"`
			SourceType   string `json:"sourceType,omitempty"`
			Source       string `json:"source,omitempty"`
			SkillPath    string `json:"skillPath"`
			Surface      string `json:"surface,omitempty"`
			ComputedHash string `json:"computedHash"`
			PinnedCommit string `json:"pinnedCommit,omitempty"`
			InstalledAt  string `json:"installedAt,omitempty"`
		}{e.Slug, e.RegistryRef, e.SourceType, e.Source, e.SkillPath, e.Surface, e.ComputedHash, e.PinnedCommit, e.InstalledAt})
		if err != nil {
			return nil, err
		}
		var knownMap map[string]json.RawMessage
		if err := json.Unmarshal(knownBuf, &knownMap); err != nil {
			return nil, err
		}
		for k, v := range knownMap {
			m[k] = v
		}
		entries = append(entries, m)
	}
	return json.Marshal(entries)
}

func mergeExtra(base, overlay RawExtra) RawExtra {
	if base == nil && overlay == nil {
		return nil
	}
	merged := RawExtra{}
	for k, v := range base {
		merged[k] = v
	}
	for k, v := range overlay {
		merged[k] = v
	}
	return merged
}
