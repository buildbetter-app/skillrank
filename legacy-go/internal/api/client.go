// Package api is a minimal authenticated HTTP client for registry writes. Reads
// are anonymous (handled directly in the registry package). A token is only
// needed to publish, rate, or review — the core experience (search/install/eval)
// requires no account. The token comes from SKILLRANK_TOKEN or ~/.skillrank/auth.json.
package api

import (
	"encoding/json"
	"io"
	"net/http"
	"os"
	"strings"

	"github.com/buildbetter/skillrank/internal/config"
)

// Client carries a resolved bearer token for authenticated requests.
type Client struct {
	BaseURL    string
	Token      string
	HTTPClient *http.Client
}

// NewWithStoredAuthRefresh resolves a stored token (env or ~/.skillrank/auth.json)
// and returns a client. The name mirrors the embedded-in-bb variant so the
// registry package is agnostic to which harness it runs in.
func NewWithStoredAuthRefresh(baseURL string, token string, httpClient *http.Client) Client {
	if httpClient == nil {
		httpClient = http.DefaultClient
	}
	resolved := strings.TrimSpace(token)
	if resolved == "" {
		resolved = resolveToken()
	}
	return Client{
		BaseURL:    strings.TrimRight(baseURL, "/"),
		Token:      resolved,
		HTTPClient: httpClient,
	}
}

// NewRequest builds a request with the bearer token attached when present.
func (c Client) NewRequest(method, path string, body io.Reader) (*http.Request, error) {
	req, err := http.NewRequest(method, c.BaseURL+path, body)
	if err != nil {
		return nil, err
	}
	if c.Token != "" {
		req.Header.Set("Authorization", "Bearer "+c.Token)
	}
	return req, nil
}

// Do sends the request.
func (c *Client) Do(req *http.Request) (*http.Response, error) {
	return c.HTTPClient.Do(req)
}

type storedAuth struct {
	Token string `json:"token"`
}

func resolveToken() string {
	if v := strings.TrimSpace(os.Getenv("SKILLRANK_TOKEN")); v != "" {
		return v
	}
	path, err := config.AuthPath()
	if err != nil {
		return ""
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return ""
	}
	var stored storedAuth
	if err := json.Unmarshal(data, &stored); err != nil {
		return ""
	}
	return strings.TrimSpace(stored.Token)
}

// SaveToken writes a token to ~/.skillrank/auth.json (used by `skillrank login`).
func SaveToken(token string) error {
	path, err := config.AuthPath()
	if err != nil {
		return err
	}
	buf, err := json.MarshalIndent(storedAuth{Token: strings.TrimSpace(token)}, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, buf, 0o600)
}
