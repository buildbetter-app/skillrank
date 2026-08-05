//! Registry HTTP client. Reads are anonymous (no Authorization header); writes
//! (not needed by the core install flow) would attach a token.

use crate::config;
use crate::hash::split_ref;
use crate::types::{ResolveResponse, SearchResponse, SkillDetail, SkillFacets, SuiteListResponse};
use serde::de::DeserializeOwned;

/// The registry's REST namespace, distinct from any tenant `/v3/rest/skills` routes.
pub const PATH_PREFIX: &str = "/v3/rest/skill-registry";

/// Errors from registry requests, with the cases callers special-case.
#[derive(Debug)]
pub enum ClientError {
    NotFound,
    Unauthorized,
    RateLimited { retry_after: Option<String> },
    Http { status: u16, body: String },
    Unreachable(String),
    Parse(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::NotFound => write!(f, "not found"),
            ClientError::Unauthorized => write!(f, "not signed in"),
            ClientError::RateLimited { retry_after } => match retry_after {
                Some(s) => write!(f, "rate limited by the registry; retry after {s} seconds"),
                None => write!(f, "rate limited by the registry; please retry shortly"),
            },
            ClientError::Http { status, body } => {
                write!(f, "registry request failed: HTTP {status}: {}", body.trim())
            }
            ClientError::Unreachable(e) => write!(f, "registry unreachable: {e}"),
            ClientError::Parse(e) => write!(f, "could not parse registry response: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl ClientError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, ClientError::NotFound)
    }
}

/// Parameters for [`Client::search`].
#[derive(Debug, Default, Clone)]
pub struct SearchOptions {
    pub query: String,
    pub stack: String,
    pub agent: String,
    pub category: String,
    /// One of the `scan_tiers` values from [`Client::skill_facets`], e.g. `"safe"`.
    /// Empty means every tier.
    pub scan_tier: String,
    pub sort: String,
    pub limit: u32,
    pub cursor: String,
}

/// Talks to the registry. Reads are anonymous.
pub struct Client {
    pub base_url: String,
}

impl Client {
    /// Resolve the configured API base URL (respecting an override) and return a client.
    pub fn new(base_url_override: Option<&str>) -> Self {
        let base = match base_url_override {
            Some(s) if !s.trim().is_empty() => s.trim().trim_end_matches('/').to_string(),
            _ => config::configured_api_base_url(),
        };
        Client { base_url: base }
    }

    fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, ClientError> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = ureq::get(&url).set("Accept", "application/json");
        for (k, v) in query {
            req = req.query(k, v);
        }
        do_json(req.call())
    }

    /// Search the registry.
    pub fn search(&self, opts: &SearchOptions) -> Result<SearchResponse, ClientError> {
        let limit = if opts.limit == 0 {
            "20".to_string()
        } else {
            opts.limit.to_string()
        };
        let mut query: Vec<(&str, &str)> = Vec::new();
        if !opts.query.is_empty() {
            query.push(("q", &opts.query));
        }
        if !opts.stack.is_empty() {
            query.push(("stack", &opts.stack));
        }
        if !opts.agent.is_empty() {
            query.push(("agent", &opts.agent));
        }
        if !opts.category.is_empty() {
            query.push(("category", &opts.category));
        }
        if !opts.scan_tier.is_empty() {
            query.push(("scan_tier", &opts.scan_tier));
        }
        if !opts.sort.is_empty() {
            query.push(("sort", &opts.sort));
        }
        query.push(("limit", &limit));
        self.get_json(&format!("{PATH_PREFIX}/skills"), &query)
    }

    /// Fetch a skill's full detail page.
    pub fn show(&self, slug: &str) -> Result<SkillDetail, ClientError> {
        self.get_json(&format!("{PATH_PREFIX}/skills/{}", encode_path(slug)), &[])
    }

    /// Return install coordinates for a ref (slug or slug@version).
    pub fn resolve(&self, reference: &str) -> Result<ResolveResponse, ClientError> {
        let (slug, version) = split_ref(reference);
        let path = format!("{PATH_PREFIX}/skills/{}/resolve", encode_path(&slug));
        if version.is_empty() {
            self.get_json(&path, &[])
        } else {
            self.get_json(&path, &[("version", version.as_str())])
        }
    }

    /// Fetch the catalog's real filter vocabulary — the categories, stacks, and
    /// scan tiers that exist, with the number of skills behind each.
    ///
    /// Filter UIs should be built from this rather than from a hardcoded option
    /// list: every value here matches at least one skill, and its `count` is the
    /// `total` the equivalent [`Client::search`] returns.
    pub fn skill_facets(&self) -> Result<SkillFacets, ClientError> {
        self.get_json(&format!("{PATH_PREFIX}/skills/facets"), &[])
    }

    /// List the registry's eval suites: ids, versions, task counts, and reference
    /// environments, without the task bodies. This is how a caller discovers a
    /// suite id instead of being asked to type one.
    pub fn list_eval_suites(&self) -> Result<SuiteListResponse, ClientError> {
        self.get_json(&format!("{PATH_PREFIX}/eval-suites"), &[])
    }

    /// Fetch an eval suite's public definition.
    pub fn get_suite(&self, id: &str) -> Result<crate::types::Suite, ClientError> {
        self.get_json(
            &format!("{PATH_PREFIX}/eval-suites/{}", encode_path(id)),
            &[],
        )
    }

    /// Fetch the private verifier scripts for a suite version (authenticated,
    /// runner-only endpoint — never part of the public suite contract).
    pub fn fetch_verifiers(
        &self,
        suite_id: &str,
        version: &str,
    ) -> Result<std::collections::HashMap<String, String>, ClientError> {
        let path = format!(
            "{PATH_PREFIX}/eval-suites/{}/verifiers",
            encode_path(suite_id)
        );
        if version.is_empty() {
            self.get_authenticated(&path, &[])
        } else {
            self.get_authenticated(&path, &[("version", version)])
        }
    }

    /// Publish an eval result bundle (authenticated).
    pub fn submit_bundle(
        &self,
        bundle: &crate::types::EvalBundle,
    ) -> Result<crate::types::IngestResponse, ClientError> {
        self.post_authenticated(&format!("{PATH_PREFIX}/eval-results"), bundle)
    }

    /// Provision an anonymous registry account and return its token.
    ///
    /// No identity is involved: the registry mints an opaque token so a client
    /// can publish without a signup flow. Anonymous results are always
    /// Self-reported — only verified accounts count toward the independent
    /// corroboration behind Community-reported — so this is a convenience, not a
    /// way to earn trust. Callers store the token like `login` does.
    pub fn provision_anonymous_token(&self) -> Result<crate::types::TokenGrant, ClientError> {
        let url = format!("{}{PATH_PREFIX}/auth/tokens", self.base_url);
        do_json(
            ureq::post(&url)
                .set("Accept", "application/json")
                .send_json(serde_json::json!({ "kind": "anonymous" })),
        )
    }

    /// Begin a GitHub device authorization.
    ///
    /// This is the only way to earn a *verified* account, which is what makes a
    /// published result count toward independent corroboration. It exists so no
    /// client ever asks a human to paste a token: show `user_code`, open
    /// `verification_uri`, then poll [`Client::poll_device_token`].
    pub fn start_device_authorization(
        &self,
    ) -> Result<crate::types::DeviceAuthorization, ClientError> {
        let url = format!("{}{PATH_PREFIX}/auth/device", self.base_url);
        do_json(ureq::post(&url).set("Accept", "application/json").call())
    }

    /// Poll a device authorization once.
    ///
    /// The registry answers 202 while the human has not finished in the browser
    /// and 201 with the token once they have, so "still waiting" is a normal
    /// return rather than an error. Honour the returned `interval` — it is the
    /// registry relaying GitHub's `slow_down`, and ignoring it gets the poll
    /// rate-limited.
    pub fn poll_device_token(
        &self,
        device_code: &str,
    ) -> Result<crate::types::DevicePoll, ClientError> {
        let url = format!("{}{PATH_PREFIX}/auth/tokens", self.base_url);
        let response = ureq::post(&url)
            .set("Accept", "application/json")
            .send_json(serde_json::json!({ "kind": "github", "device_code": device_code }));
        match response {
            Ok(resp) if resp.status() == 202 => {
                #[derive(serde::Deserialize)]
                struct Waiting {
                    #[serde(default)]
                    interval: u64,
                }
                let waiting: Waiting = resp
                    .into_json()
                    .map_err(|e| ClientError::Parse(e.to_string()))?;
                Ok(crate::types::DevicePoll::Pending {
                    interval: waiting.interval,
                })
            }
            Ok(resp) => resp
                .into_json::<crate::types::TokenGrant>()
                .map(|grant| crate::types::DevicePoll::Granted(Box::new(grant)))
                .map_err(|e| ClientError::Parse(e.to_string())),
            // Only `Err` reaches here; `do_json` turns it into the right
            // ClientError (401/404/429/transport) rather than duplicating that.
            failed => do_json::<crate::types::TokenGrant>(failed)
                .map(|grant| crate::types::DevicePoll::Granted(Box::new(grant))),
        }
    }

    /// Subscribe an email to occasional skill updates (unauthenticated,
    /// best-effort). The registry stores only the address; no account is created.
    pub fn subscribe_email(&self, email: &str) -> Result<(), ClientError> {
        let url = format!("{}{PATH_PREFIX}/subscribe", self.base_url);
        match ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_json(serde_json::json!({ "email": email }))
        {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, resp)) => Err(ClientError::Http {
                status: code,
                body: resp.into_string().unwrap_or_default(),
            }),
            Err(ureq::Error::Transport(t)) => Err(ClientError::Unreachable(t.to_string())),
        }
    }

    fn get_authenticated<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, ClientError> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = ureq::get(&url).set("Accept", "application/json");
        if let Some(token) = resolve_token() {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
        for (k, v) in query {
            req = req.query(k, v);
        }
        do_json(req.call())
    }

    fn post_authenticated<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = ureq::post(&url).set("Accept", "application/json");
        if let Some(token) = resolve_token() {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
        do_json(req.send_json(serde_json::to_value(body).unwrap_or(serde_json::Value::Null)))
    }

    /// Download SKILL.md content from a raw URL (source-mode skills whose content
    /// the registry did not inline).
    pub fn fetch_raw_content(&self, raw_url: &str) -> Result<String, ClientError> {
        match ureq::get(raw_url).call() {
            Ok(resp) => resp
                .into_string()
                .map_err(|e| ClientError::Parse(e.to_string())),
            Err(ureq::Error::Status(code, resp)) => Err(ClientError::Http {
                status: code,
                body: resp.into_string().unwrap_or_default(),
            }),
            Err(ureq::Error::Transport(t)) => Err(ClientError::Unreachable(t.to_string())),
        }
    }
}

/// Percent-encode a path segment (RFC 3986 unreserved kept; everything else,
/// including `/`, becomes %XX) so slugs like `owner/skill` survive routing.
fn encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Convert a ureq call result into a typed JSON result with mapped error cases.
fn do_json<T: DeserializeOwned>(
    result: Result<ureq::Response, ureq::Error>,
) -> Result<T, ClientError> {
    match result {
        Ok(resp) => resp
            .into_json::<T>()
            .map_err(|e| ClientError::Parse(e.to_string())),
        Err(ureq::Error::Status(code, resp)) => {
            let retry_after = resp.header("Retry-After").map(|s| s.to_string());
            let body = resp.into_string().unwrap_or_default();
            Err(match code {
                404 => ClientError::NotFound,
                401 => ClientError::Unauthorized,
                429 => ClientError::RateLimited { retry_after },
                _ => ClientError::Http { status: code, body },
            })
        }
        Err(ureq::Error::Transport(t)) => Err(ClientError::Unreachable(t.to_string())),
    }
}

/// Resolve a registry token from SKILLRANK_TOKEN or ~/.skillrank/auth.json.
fn resolve_token() -> Option<String> {
    if let Ok(t) = std::env::var("SKILLRANK_TOKEN") {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    let path = crate::config::auth_path().ok()?;
    let data = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&data).ok()?;
    v.get("token")
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DevicePoll;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Answer the first request with one canned response and hand back a base
    /// URL to point a `Client` at. Stdlib only — the device poll's whole
    /// subtlety is that a 202 and a 201 are both success statuses meaning
    /// opposite things, which no type check can catch and no amount of mocking
    /// the parse layer would exercise.
    fn serve_once(status: &str, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut scratch = [0u8; 4096];
                let _ = stream.read(&mut scratch);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        format!("http://{address}")
    }

    fn client_at(base_url: String) -> Client {
        Client { base_url }
    }

    #[test]
    fn a_waiting_device_poll_is_an_outcome_rather_than_an_error() {
        // 202 means the human simply has not finished in the browser yet.
        // Treating it as an error would abort a flow that is working.
        let base = serve_once("202 Accepted", r#"{"status":"pending","interval":5}"#);
        match client_at(base).poll_device_token("dev-code").unwrap() {
            DevicePoll::Pending { interval } => assert_eq!(interval, 5),
            DevicePoll::Granted(_) => panic!("a pending poll must not report a token"),
        }
    }

    #[test]
    fn slow_down_raises_the_interval_the_caller_must_honour() {
        // The registry relays GitHub's slow_down as a bigger interval; a client
        // that ignores it gets rate-limited out of its own sign-in.
        let base = serve_once("202 Accepted", r#"{"status":"slow_down","interval":10}"#);
        match client_at(base).poll_device_token("dev-code").unwrap() {
            DevicePoll::Pending { interval } => assert_eq!(interval, 10),
            DevicePoll::Granted(_) => panic!("slow_down is still waiting"),
        }
    }

    #[test]
    fn an_approved_device_poll_yields_the_verified_grant() {
        let base = serve_once(
            "201 Created",
            r#"{"token":"srk_abc","kind":"github","account_id":"gh_1","verified":true}"#,
        );
        match client_at(base).poll_device_token("dev-code").unwrap() {
            DevicePoll::Granted(grant) => {
                assert_eq!(grant.token, "srk_abc");
                assert_eq!(grant.kind, "github");
                assert!(grant.verified, "github sign-in is what earns verified");
            }
            DevicePoll::Pending { .. } => panic!("an approved poll must return the token"),
        }
    }

    #[test]
    fn a_registry_without_github_configured_says_so_instead_of_hanging() {
        // 503 must surface as an error, not as another "keep polling" — else the
        // UI spins forever against a registry that will never grant anything.
        let base = serve_once(
            "503 Service Unavailable",
            r#"{"error":"github sign-in is not configured on this registry"}"#,
        );
        let failure = client_at(base).poll_device_token("dev-code").unwrap_err();
        match failure {
            ClientError::Http { status, body } => {
                assert_eq!(status, 503);
                assert!(body.contains("not configured"));
            }
            other => panic!("expected an HTTP error, got {other}"),
        }
    }

    #[test]
    fn starting_a_device_authorization_reads_back_the_user_facing_code() {
        let base = serve_once(
            "200 OK",
            r#"{"device_code":"secret","user_code":"WDJB-MJHT","verification_uri":"https://github.com/login/device","interval":5,"expires_in":900}"#,
        );
        let started = client_at(base).start_device_authorization().unwrap();
        assert_eq!(started.user_code, "WDJB-MJHT");
        assert_eq!(started.verification_uri, "https://github.com/login/device");
        assert_eq!(started.interval, 5);
        assert_eq!(started.device_code, "secret");
    }
}
