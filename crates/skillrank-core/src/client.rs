//! Registry HTTP client. Reads are anonymous (no Authorization header); writes
//! (not needed by the core install flow) would attach a token.

use crate::config;
use crate::hash::split_ref;
use crate::types::{ResolveResponse, SearchResponse, SkillDetail};
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
        match req.call() {
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
        if !opts.sort.is_empty() {
            query.push(("sort", &opts.sort));
        }
        query.push(("limit", &limit));
        self.get_json(&format!("{PATH_PREFIX}/skills"), &query)
    }

    /// Fetch a skill's full detail page.
    pub fn show(&self, slug: &str) -> Result<SkillDetail, ClientError> {
        self.get_json(
            &format!("{PATH_PREFIX}/skills/{}", encode_path(slug)),
            &[],
        )
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
