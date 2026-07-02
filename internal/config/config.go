// Package config resolves the registry API base URL and local paths for the
// standalone skillrank CLI. It intentionally does NOT depend on any BuildBetter
// configuration: the tool works on its own.
package config

import (
	"os"
	"path/filepath"
	"strings"
)

// DefaultAPIBaseURL is the hosted SkillRank registry. Override with
// SKILLRANK_API_URL for self-hosted registries or local development.
const DefaultAPIBaseURL = "https://api.skillrank.dev"

// ConfiguredAPIBaseURL returns the registry base URL, honoring SKILLRANK_API_URL.
func ConfiguredAPIBaseURL() (string, error) {
	if v := strings.TrimSpace(os.Getenv("SKILLRANK_API_URL")); v != "" {
		return strings.TrimRight(v, "/"), nil
	}
	return DefaultAPIBaseURL, nil
}

// Home returns the skillrank config directory (~/.skillrank), creating it.
func Home() (string, error) {
	base := strings.TrimSpace(os.Getenv("SKILLRANK_HOME"))
	if base == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			return "", err
		}
		base = filepath.Join(home, ".skillrank")
	}
	if err := os.MkdirAll(base, 0o755); err != nil {
		return "", err
	}
	return base, nil
}

// AuthPath is where the (optional) registry token is stored.
func AuthPath() (string, error) {
	home, err := Home()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, "auth.json"), nil
}
