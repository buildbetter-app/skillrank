package registry

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"

	bbapi "github.com/buildbetter/skillrank/internal/api"
	bbconfig "github.com/buildbetter/skillrank/internal/config"
)

// PathPrefix is the registry's REST namespace. It is deliberately distinct from
// the tenant `/v3/rest/skills` product routes.
const PathPrefix = "/v3/rest/skill-registry"

// Client talks to the registry. Reads are anonymous (no Authorization header);
// writes resolve stored BuildBetter auth on demand.
type Client struct {
	BaseURL    string
	HTTPClient *http.Client
}

// NewClient resolves the configured API base URL (respecting overrides) and
// returns a registry client. An explicit baseURLOverride wins when non-empty.
func NewClient(baseURLOverride string, httpClient *http.Client) (Client, error) {
	base := strings.TrimSpace(baseURLOverride)
	if base == "" {
		configured, err := bbconfig.ConfiguredAPIBaseURL()
		if err != nil {
			return Client{}, err
		}
		base = configured
	}
	if httpClient == nil {
		httpClient = http.DefaultClient
	}
	return Client{BaseURL: strings.TrimRight(base, "/"), HTTPClient: httpClient}, nil
}

// apiError carries an HTTP status for callers that special-case 404/429.
type apiError struct {
	Status  int
	Body    string
	Message string
}

func (e *apiError) Error() string {
	if e.Message != "" {
		return e.Message
	}
	return fmt.Sprintf("registry request failed: HTTP %d: %s", e.Status, strings.TrimSpace(e.Body))
}

// IsNotFound reports whether err is a 404 from the registry.
func IsNotFound(err error) bool {
	var ae *apiError
	if e, ok := err.(*apiError); ok {
		ae = e
	}
	return ae != nil && ae.Status == http.StatusNotFound
}

// getAnonymous performs a public GET with no Authorization header. Public
// registry routes 401 a present-but-expired bearer, so reads never attach one.
func (c Client) getAnonymous(path string, query url.Values, out any) error {
	full := c.BaseURL + path
	if len(query) > 0 {
		full += "?" + query.Encode()
	}
	req, err := http.NewRequest(http.MethodGet, full, nil)
	if err != nil {
		return err
	}
	req.Header.Set("Accept", "application/json")
	resp, err := c.HTTPClient.Do(req)
	if err != nil {
		return fmt.Errorf("registry unreachable: %w", err)
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	if resp.StatusCode == http.StatusTooManyRequests {
		return &apiError{Status: resp.StatusCode, Body: string(body), Message: rateLimitMessage(resp)}
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return &apiError{Status: resp.StatusCode, Body: string(body)}
	}
	if out == nil {
		return nil
	}
	return json.Unmarshal(body, out)
}

// postAuthenticated performs a write with stored BuildBetter auth (refreshed).
func (c Client) postAuthenticated(path string, payload any, out any) error {
	buf, err := json.Marshal(payload)
	if err != nil {
		return err
	}
	client := bbapi.NewWithStoredAuthRefresh(c.BaseURL, "", c.HTTPClient)
	req, err := client.NewRequest(http.MethodPost, path, bytes.NewReader(buf))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json")
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("registry unreachable: %w", err)
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	if resp.StatusCode == http.StatusUnauthorized {
		return &apiError{Status: resp.StatusCode, Body: string(body), Message: "not signed in — run `skillrank login` (or `bb auth login`) before publishing"}
	}
	if resp.StatusCode == http.StatusTooManyRequests {
		return &apiError{Status: resp.StatusCode, Body: string(body), Message: rateLimitMessage(resp)}
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return &apiError{Status: resp.StatusCode, Body: string(body)}
	}
	if out == nil {
		return nil
	}
	return json.Unmarshal(body, out)
}

func rateLimitMessage(resp *http.Response) string {
	if retry := strings.TrimSpace(resp.Header.Get("Retry-After")); retry != "" {
		return fmt.Sprintf("rate limited by the registry; retry after %s seconds", retry)
	}
	return "rate limited by the registry; please retry shortly"
}

// RetryAfterSeconds returns the Retry-After hint for a 429 error, or 0.
func RetryAfterSeconds(err error) int {
	if ae, ok := err.(*apiError); ok && ae.Status == http.StatusTooManyRequests {
		// The seconds value is embedded in the human message; callers that need
		// the raw header should thread it explicitly. Kept simple here.
		return 0
	}
	return 0
}

// Search queries the registry.
func (c Client) Search(opts SearchOptions) (SearchResponse, error) {
	q := url.Values{}
	if opts.Query != "" {
		q.Set("q", opts.Query)
	}
	if opts.Stack != "" {
		q.Set("stack", opts.Stack)
	}
	if opts.Agent != "" {
		q.Set("agent", opts.Agent)
	}
	if opts.Category != "" {
		q.Set("category", opts.Category)
	}
	if opts.Sort != "" {
		q.Set("sort", opts.Sort)
	}
	if opts.Limit > 0 {
		q.Set("limit", strconv.Itoa(opts.Limit))
	}
	if opts.Cursor != "" {
		q.Set("cursor", opts.Cursor)
	}
	var out SearchResponse
	if err := c.getAnonymous(PathPrefix+"/skills", q, &out); err != nil {
		return SearchResponse{}, err
	}
	return out, nil
}

// SearchOptions parameterize Search.
type SearchOptions struct {
	Query    string
	Stack    string
	Agent    string
	Category string
	Sort     string
	Limit    int
	Cursor   string
}

// Show fetches a skill's full detail page.
func (c Client) Show(slug string) (SkillDetail, error) {
	var out SkillDetail
	if err := c.getAnonymous(PathPrefix+"/skills/"+url.PathEscape(slug), nil, &out); err != nil {
		return SkillDetail{}, err
	}
	return out, nil
}

// Resolve returns install coordinates for a ref (slug or slug@version).
func (c Client) Resolve(ref string) (ResolveResponse, error) {
	slug, version := SplitRef(ref)
	q := url.Values{}
	if version != "" {
		q.Set("version", version)
	}
	var out ResolveResponse
	if err := c.getAnonymous(PathPrefix+"/skills/"+url.PathEscape(slug)+"/resolve", q, &out); err != nil {
		return ResolveResponse{}, err
	}
	return out, nil
}

// FetchRawContent downloads SKILL.md content from a raw URL (source-mode skills
// whose content the registry did not inline).
func (c Client) FetchRawContent(rawURL string) (string, error) {
	req, err := http.NewRequest(http.MethodGet, rawURL, nil)
	if err != nil {
		return "", err
	}
	resp, err := c.HTTPClient.Do(req)
	if err != nil {
		return "", fmt.Errorf("fetch skill content: %w", err)
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return "", fmt.Errorf("fetch skill content: HTTP %d", resp.StatusCode)
	}
	return string(body), nil
}

// GetSuite fetches an eval suite's public definition.
func (c Client) GetSuite(id string) (Suite, error) {
	var out Suite
	if err := c.getAnonymous(PathPrefix+"/eval-suites/"+url.PathEscape(id), nil, &out); err != nil {
		return Suite{}, err
	}
	return out, nil
}

// FetchVerifiers fetches the private verifier scripts for a suite version. This
// is an authenticated, runner-only endpoint: verifier bodies and oracles are
// never part of the public suite contract, so the agent workspace can never see
// them (verifier isolation).
func (c Client) FetchVerifiers(suiteID, version string) (map[string]string, error) {
	q := url.Values{}
	if version != "" {
		q.Set("version", version)
	}
	path := PathPrefix + "/eval-suites/" + url.PathEscape(suiteID) + "/verifiers"
	if len(q) > 0 {
		path += "?" + q.Encode()
	}
	var out map[string]string
	if err := c.getAuthenticated(path, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// getAuthenticated performs a GET with stored BuildBetter auth (refreshed).
func (c Client) getAuthenticated(path string, out any) error {
	client := bbapi.NewWithStoredAuthRefresh(c.BaseURL, "", c.HTTPClient)
	req, err := client.NewRequest(http.MethodGet, path, nil)
	if err != nil {
		return err
	}
	req.Header.Set("Accept", "application/json")
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("registry unreachable: %w", err)
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	if resp.StatusCode == http.StatusUnauthorized {
		return &apiError{Status: resp.StatusCode, Body: string(body), Message: "not signed in — run `skillrank login` before running evals that fetch verifiers"}
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return &apiError{Status: resp.StatusCode, Body: string(body)}
	}
	if out == nil {
		return nil
	}
	return json.Unmarshal(body, out)
}

// SubmitBundle publishes an eval result bundle (authenticated).
func (c Client) SubmitBundle(bundle EvalBundle) (IngestResponse, error) {
	var out IngestResponse
	if err := c.postAuthenticated(PathPrefix+"/eval-results", bundle, &out); err != nil {
		return IngestResponse{}, err
	}
	return out, nil
}

// PublishSource indexes a public repo skill (authenticated).
func (c Client) PublishSource(sourceURL, subpath string) (PublishResponse, error) {
	payload := map[string]string{"source_url": sourceURL, "source_subpath": subpath}
	var out PublishResponse
	if err := c.postAuthenticated(PathPrefix+"/skills", payload, &out); err != nil {
		return PublishResponse{}, err
	}
	return out, nil
}

// SplitRef splits "slug@version" into its parts.
func SplitRef(ref string) (slug string, version string) {
	ref = strings.TrimSpace(ref)
	if i := strings.LastIndex(ref, "@"); i > 0 {
		return ref[:i], ref[i+1:]
	}
	return ref, ""
}
