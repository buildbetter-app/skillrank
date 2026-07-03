//! `skillrank serve` — a local registry implementing the read half of the
//! /v3/rest/skill-registry contract from a seed catalog, so search / recommend /
//! install work with no hosted backend. Same wire contract the hosted registry
//! serves, so nothing about the CLI/MCP changes.

use crate::flags::Flags;
use serde::{Deserialize, Serialize};
use skillrank_core as core;
use std::collections::HashMap;
use tiny_http::{Header, Response, Server};

const SEED_CATALOG: &str = include_str!("seed_catalog.json");

#[derive(Debug, Clone, Deserialize)]
struct CatalogEntry {
    slug: String,
    display_name: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    stacks: Vec<String>,
    #[serde(default)]
    source_url: String,
    #[serde(default)]
    summary: String,
    content: String,
    #[serde(skip)]
    hash: String,
}

pub fn run(args: &[String]) -> i32 {
    let f = Flags::parse(args);
    let port: u16 = f.value("port").parse().unwrap_or(8899);

    let raw = if !f.value("catalog").is_empty() {
        match std::fs::read_to_string(f.value("catalog")) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skillrank serve: read catalog: {e}");
                return 1;
            }
        }
    } else {
        SEED_CATALOG.to_string()
    };
    let mut entries: Vec<CatalogEntry> = match serde_json::from_str(&raw) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("skillrank serve: parse catalog: {e}");
            return 1;
        }
    };
    for e in entries.iter_mut() {
        e.hash = core::compute_content_hash(&e.content);
    }
    let index: HashMap<String, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.slug.clone(), i))
        .collect();
    let server = ServerState { entries, index };

    let addr = format!("0.0.0.0:{port}");
    let http = match Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skillrank serve: {e}");
            return 1;
        }
    };
    println!(
        "skillrank registry serving {} skills on http://localhost:{port}",
        server.entries.len()
    );
    println!("Point your CLI/agent at it:  export SKILLRANK_API_URL=http://localhost:{port}");

    let json_header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    for request in http.incoming_requests() {
        let (path, query) = split_url(request.url());
        let (status, body) = server.route(&path, &query);
        let response = Response::from_string(body)
            .with_status_code(status)
            .with_header(json_header.clone());
        let _ = request.respond(response);
    }
    0
}

struct ServerState {
    entries: Vec<CatalogEntry>,
    index: HashMap<String, usize>,
}

impl ServerState {
    fn route(&self, path: &str, query: &HashMap<String, String>) -> (u16, String) {
        let skills = format!("{}/skills", core::PATH_PREFIX);
        let skills_prefix = format!("{skills}/");
        if path == "/healthz" {
            return (200, "ok".to_string());
        }
        if path == skills {
            return (200, self.search(query));
        }
        if let Some(rest) = path.strip_prefix(&skills_prefix) {
            if let Some(slug) = rest.strip_suffix("/resolve") {
                return self.resolve(slug);
            }
            return self.show(rest);
        }
        (404, json_error("not found"))
    }

    fn search(&self, query: &HashMap<String, String>) -> String {
        let q = query.get("q").cloned().unwrap_or_default().to_lowercase();
        let stack = query.get("stack").cloned().unwrap_or_default().to_lowercase();
        let category = query.get("category").cloned().unwrap_or_default().to_lowercase();
        let limit: usize = query
            .get("limit")
            .and_then(|s| s.parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(20);

        let mut items: Vec<core::SkillSummary> = Vec::new();
        for e in &self.entries {
            if !stack.is_empty() && !e.stacks.iter().any(|s| s.eq_ignore_ascii_case(&stack)) {
                continue;
            }
            if !category.is_empty() && !e.category.eq_ignore_ascii_case(&category) {
                continue;
            }
            if !q.is_empty() && !matches_query(e, &q) {
                continue;
            }
            items.push(e.to_summary());
        }
        items.sort_by(|a, b| a.slug.cmp(&b.slug));
        items.truncate(limit);
        let total = items.len() as i64;
        serde_json::to_string(&core::SearchResponse {
            items,
            total,
            ..Default::default()
        })
        .unwrap()
    }

    fn show(&self, slug: &str) -> (u16, String) {
        let Some(&i) = self.index.get(slug) else {
            return (404, json_error("not found"));
        };
        let e = &self.entries[i];
        let detail = core::SkillDetail {
            summary: e.to_summary(),
            versions: vec![core::SkillVersion {
                content_hash: e.hash.clone(),
                scan_tier: core::ScanTier::Safe,
                ..Default::default()
            }],
            eval_cells: vec![],
        };
        (200, serde_json::to_string(&detail).unwrap())
    }

    fn resolve(&self, slug: &str) -> (u16, String) {
        let Some(&i) = self.index.get(slug) else {
            return (404, json_error("not found"));
        };
        let e = &self.entries[i];
        let resolved = core::ResolveResponse {
            slug: e.slug.clone(),
            version: e.hash.clone(),
            source_type: "github".into(),
            source_url: e.source_url.clone(),
            content_hash: e.hash.clone(),
            scan_tier: core::ScanTier::Safe,
            inline_content: e.content.clone(),
            ..Default::default()
        };
        (200, serde_json::to_string(&resolved).unwrap())
    }
}

impl CatalogEntry {
    fn to_summary(&self) -> core::SkillSummary {
        core::SkillSummary {
            slug: self.slug.clone(),
            display_name: self.display_name.clone(),
            category: self.category.clone(),
            stacks: self.stacks.clone(),
            source_type: "github".into(),
            source_url: self.source_url.clone(),
            latest_version: self.hash.clone(),
            scan_tier: core::ScanTier::Safe,
            rating_count: 0,
            summary: self.summary.clone(),
            ..Default::default()
        }
    }
}

#[derive(Serialize)]
struct ErrBody {
    error: String,
}

fn json_error(msg: &str) -> String {
    serde_json::to_string(&ErrBody { error: msg.to_string() }).unwrap()
}

/// matches_query is true when every whitespace word of the query is a substring
/// of the skill's searchable text, or the whole query (spaces/dashes removed) is a
/// contiguous substring. Requiring all words keeps "front end" matching frontend
/// skills without matching "backend"/"dependency" on the stray word "end".
fn matches_query(e: &CatalogEntry, query: &str) -> bool {
    let hay = format!(
        "{} {} {} {} {}",
        e.slug,
        e.display_name,
        e.summary,
        e.category,
        e.stacks.join(" ")
    )
    .to_lowercase();
    let collapsed_q = strip_sep(query);
    if !collapsed_q.is_empty() && strip_sep(&hay).contains(&collapsed_q) {
        return true;
    }
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }
    words.iter().all(|w| hay.contains(w))
}

fn strip_sep(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, ' ' | '-' | '_')).collect()
}

/// Split a request target into (decoded path, query map). Percent-decodes the
/// path so slugs sent as %2F-encoded arrive with real slashes.
fn split_url(url: &str) -> (String, HashMap<String, String>) {
    let (path, query) = match url.split_once('?') {
        Some((p, q)) => (p, q),
        None => (url, ""),
    };
    let mut params = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        params.insert(percent_decode(k), percent_decode(v));
    }
    (percent_decode(path), params)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(slug: &str, cat: &str, stacks: &[&str]) -> CatalogEntry {
        CatalogEntry {
            slug: slug.into(),
            display_name: slug.into(),
            category: cat.into(),
            stacks: stacks.iter().map(|s| s.to_string()).collect(),
            source_url: String::new(),
            summary: String::new(),
            content: "---\nname: x\n---\nbody".into(),
            hash: String::new(),
        }
    }

    #[test]
    fn front_end_matches_frontend_not_backend() {
        let fe = entry("x/react", "frontend", &["react"]);
        let be = entry("x/fastapi", "backend", &["fastapi", "python"]);
        assert!(matches_query(&fe, "front end"));
        assert!(!matches_query(&be, "front end"));
    }

    #[test]
    fn split_url_decodes_slug_and_query() {
        let (path, params) = split_url("/v3/rest/skill-registry/skills/owner%2Fname/resolve?version=1&q=a+b");
        assert_eq!(path, "/v3/rest/skill-registry/skills/owner/name/resolve");
        assert_eq!(params.get("version").unwrap(), "1");
        assert_eq!(params.get("q").unwrap(), "a b");
    }

    #[test]
    fn resolve_hash_matches_its_own_content() {
        let mut e = entry("x/y", "frontend", &["react"]);
        e.hash = skillrank_core::compute_content_hash(&e.content);
        let state = ServerState { entries: vec![e], index: [("x/y".to_string(), 0usize)].into_iter().collect() };
        let (status, body) = state.resolve("x/y");
        assert_eq!(status, 200);
        let resolved: skillrank_core::ResolveResponse = serde_json::from_str(&body).unwrap();
        assert!(skillrank_core::hashes_equal(
            &skillrank_core::compute_content_hash(&resolved.inline_content),
            &resolved.content_hash
        ));
    }
}
