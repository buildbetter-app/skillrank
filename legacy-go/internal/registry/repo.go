package registry

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	skills "github.com/buildbetter/skillrank/internal/skills"
)

// RepoRoot returns the git top-level for cwd, falling back to cwd when not in a
// git repo.
func RepoRoot(cwd string) string {
	if strings.TrimSpace(cwd) == "" {
		if wd, err := os.Getwd(); err == nil {
			cwd = wd
		}
	}
	cmd := exec.Command("git", "rev-parse", "--show-toplevel")
	cmd.Dir = cwd
	out, err := cmd.Output()
	if err == nil {
		if root := strings.TrimSpace(string(out)); root != "" {
			return root
		}
	}
	return cwd
}

// HeadCommit returns the current HEAD short SHA, or "".
func HeadCommit(repoRoot string) string {
	cmd := exec.Command("git", "rev-parse", "HEAD")
	cmd.Dir = repoRoot
	out, err := cmd.Output()
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(out))
}

// ResolveSurface chooses the skill surface directory for install. An explicit
// override wins; otherwise the first existing SupportedDirectories entry is used;
// otherwise `.claude/skills` is created.
func ResolveSurface(repoRoot, override string) (relative string, absolute string, err error) {
	if o := strings.TrimSpace(override); o != "" {
		rel := filepath.ToSlash(o)
		return rel, filepath.Join(repoRoot, filepath.FromSlash(rel)), nil
	}
	discovery, err := skills.Discover(repoRoot)
	if err == nil && discovery.Surface != nil {
		return discovery.Surface.RelativePath, discovery.Surface.AbsolutePath, nil
	}
	rel := ".claude/skills"
	return rel, filepath.Join(repoRoot, filepath.FromSlash(rel)), nil
}
