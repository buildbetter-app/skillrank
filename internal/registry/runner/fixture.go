package runner

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// GitFixtureProvider prepares a pinned-commit clone of the fixture repo into a
// fresh temp dir per trial (worktree isolation). Docker-container isolation is a
// planned v0.1+ increment; see DockerAvailable / NewFixtureProvider.
type GitFixtureProvider struct {
	GitURL string
	Commit string
	// baseCheckout caches a single clone that per-trial workspaces copy from.
	baseCheckout string
	tmpRoot      string
}

// NewFixtureProvider returns a worktree-isolation provider. Docker mode is
// selected only when an image is supplied AND docker is available; until the
// container path lands, callers get worktree isolation (Self-reported results).
func NewFixtureProvider(fx interface {
	URL() string
	Rev() string
	ImageRef() string
}) (FixtureProvider, error) {
	// Kept simple: always worktree in v0.1. Docker wiring is a follow-up.
	return &GitFixtureProvider{GitURL: fx.URL(), Commit: fx.Rev()}, nil
}

// NewGitFixtureProvider builds a worktree provider directly from coordinates.
func NewGitFixtureProvider(gitURL, commit string) *GitFixtureProvider {
	return &GitFixtureProvider{GitURL: gitURL, Commit: commit}
}

// Isolation implements FixtureProvider.
func (p *GitFixtureProvider) Isolation() Isolation { return IsolationWorktree }

func (p *GitFixtureProvider) ensureBaseCheckout(ctx context.Context) error {
	if p.baseCheckout != "" {
		return nil
	}
	root, err := os.MkdirTemp("", "skillrank-fixture-")
	if err != nil {
		return err
	}
	p.tmpRoot = root
	checkout := filepath.Join(root, "base")
	// Shallow clone then checkout the pinned commit for reproducibility.
	clone := exec.CommandContext(ctx, "git", "clone", "--no-checkout", p.GitURL, checkout)
	if out, err := clone.CombinedOutput(); err != nil {
		return fmt.Errorf("clone fixture %s: %w: %s", p.GitURL, err, strings.TrimSpace(string(out)))
	}
	if strings.TrimSpace(p.Commit) != "" {
		co := exec.CommandContext(ctx, "git", "checkout", p.Commit)
		co.Dir = checkout
		if out, err := co.CombinedOutput(); err != nil {
			return fmt.Errorf("checkout fixture commit %s: %w: %s", p.Commit, err, strings.TrimSpace(string(out)))
		}
	} else {
		co := exec.CommandContext(ctx, "git", "checkout", "HEAD")
		co.Dir = checkout
		_ = co.Run()
	}
	p.baseCheckout = checkout
	return nil
}

// Prepare copies the base checkout into a fresh per-trial workspace so each trial
// starts from an identical clean fixture. The workspace contains ONLY the
// fixture — never verifier content.
func (p *GitFixtureProvider) Prepare(ctx context.Context, taskID string) (string, func(), error) {
	if err := p.ensureBaseCheckout(ctx); err != nil {
		return "", func() {}, err
	}
	workDir, err := os.MkdirTemp(p.tmpRoot, "trial-")
	if err != nil {
		return "", func() {}, err
	}
	if err := copyTree(p.baseCheckout, workDir); err != nil {
		_ = os.RemoveAll(workDir)
		return "", func() {}, err
	}
	cleanup := func() { _ = os.RemoveAll(workDir) }
	return workDir, cleanup, nil
}

// Close removes the shared temp root.
func (p *GitFixtureProvider) Close() {
	if p.tmpRoot != "" {
		_ = os.RemoveAll(p.tmpRoot)
	}
}

// DockerAvailable reports whether a docker binary is on PATH.
func DockerAvailable() bool {
	_, err := exec.LookPath("docker")
	return err == nil
}

// copyTree recursively copies src into dst (dst must exist).
func copyTree(src, dst string) error {
	return filepath.Walk(src, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		rel, relErr := filepath.Rel(src, path)
		if relErr != nil {
			return relErr
		}
		if rel == "." {
			return nil
		}
		target := filepath.Join(dst, rel)
		if info.IsDir() {
			return os.MkdirAll(target, info.Mode().Perm()|0o700)
		}
		if info.Mode()&os.ModeSymlink != 0 {
			linkTarget, readErr := os.Readlink(path)
			if readErr != nil {
				return readErr
			}
			return os.Symlink(linkTarget, target)
		}
		data, readErr := os.ReadFile(path)
		if readErr != nil {
			return readErr
		}
		if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
			return err
		}
		return os.WriteFile(target, data, info.Mode().Perm())
	})
}

// ScriptVerifier runs a per-task verifier command in an isolated location. The
// verifier script/content is materialized only inside Verify (never in the
// workspace during the agent run), enforcing verifier isolation structurally.
type ScriptVerifier struct {
	// Commands maps taskID -> shell command run with the workspace as an argument
	// ($1). A zero exit code = pass.
	Commands map[string]string
	// Shell overrides the interpreter (default: "bash").
	Shell string
}

// Verify implements Verifier.
func (v *ScriptVerifier) Verify(ctx context.Context, workingDir, taskID string) (Verdict, error) {
	command, ok := v.Commands[taskID]
	if !ok || strings.TrimSpace(command) == "" {
		return Verdict{VerifierError: true}, fmt.Errorf("no verifier for task %s", taskID)
	}
	shell := v.Shell
	if shell == "" {
		shell = "bash"
	}
	// Materialize the verifier in a temp dir OUTSIDE the workspace.
	verifierDir, err := os.MkdirTemp("", "skillrank-verifier-")
	if err != nil {
		return Verdict{VerifierError: true}, err
	}
	defer os.RemoveAll(verifierDir)
	scriptPath := filepath.Join(verifierDir, "verify.sh")
	if err := os.WriteFile(scriptPath, []byte(command), 0o700); err != nil {
		return Verdict{VerifierError: true}, err
	}
	cmd := exec.CommandContext(ctx, shell, scriptPath, workingDir)
	cmd.Dir = workingDir
	if err := cmd.Run(); err != nil {
		if _, ok := err.(*exec.ExitError); ok {
			return Verdict{Pass: false}, nil
		}
		return Verdict{VerifierError: true}, err
	}
	return Verdict{Pass: true}, nil
}
