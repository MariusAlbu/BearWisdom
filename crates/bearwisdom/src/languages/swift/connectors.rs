// =============================================================================
// languages/swift/connectors.rs — Swift REST connector
//
// SwiftRestConnector:
//   Start points: URLSession (URL(string:)), Alamofire (AF.request),
//                 URLRequest(url: URL(string:)) — source-scan in plugin.
//   Stop points:  Route handler registrations in the `routes` table.
// =============================================================================

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;
use crate::types::{
    ConnectionKind, ConnectionPoint as AbstractPoint, ConnectionRole,
};

// ===========================================================================
// Plugin-facing composer
// ===========================================================================

pub fn extract_swift_connection_points(
    source: &str,
    file_path: &str,
) -> Vec<AbstractPoint> {
    let mut out = Vec::new();
    extract_swift_rest_starts_src(source, file_path, &mut out);
    out
}

// ===========================================================================
// SwiftRestConnector — starts flattened; stops on DB
// ===========================================================================

pub struct SwiftRestConnector;

impl Connector for SwiftRestConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "swift_rest",
            protocols: &[Protocol::Rest],
            languages: &["swift"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        true
    }

    fn extract(
        &self,
        conn: &Connection,
        _project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let mut points = Vec::new();
        extract_swift_rest_stops(conn, &mut points)?;
        Ok(points)
    }
}

fn extract_swift_rest_stops(conn: &Connection, out: &mut Vec<ConnectionPoint>) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT r.file_id, r.symbol_id, r.line, r.http_method,
                    COALESCE(r.resolved_route, r.route_template)
             FROM routes r
             JOIN files f ON f.id = r.file_id
             WHERE f.language = 'swift'
               AND r.http_method != '' AND r.route_template != ''",
        )
        .context("Failed to prepare Swift REST stops query")?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<u32>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .context("Failed to query Swift routes")?;

    for row in rows {
        let (file_id, symbol_id, line, method, route) =
            row.context("Failed to read Swift route row")?;
        out.push(ConnectionPoint {
            file_id,
            symbol_id,
            line: line.unwrap_or(0),
            protocol: Protocol::Rest,
            direction: FlowDirection::Stop,
            key: route,
            method: method.to_uppercase(),
            framework: String::new(),
            metadata: None,
        });
    }
    Ok(())
}

/// Swift REST client-call starts: URLSession, Alamofire, URLRequest.
pub fn extract_swift_rest_starts_src(
    source: &str,
    file_path: &str,
    out: &mut Vec<AbstractPoint>,
) {
    if swift_rest_is_test_file(file_path) {
        return;
    }
    if !source.contains("URL(") && !source.contains("AF.request") && !source.contains("URLRequest") {
        return;
    }

    let re_url = regex::Regex::new(r#"URL\s*\(\s*string\s*:\s*"(?P<url>[^"]+)""#)
        .expect("swift url regex");
    let re_af = regex::Regex::new(
        r#"AF\s*\.\s*request\s*\(\s*"(?P<url>[^"]+)"(?:\s*,\s*method\s*:\s*\.(?P<method>get|post|put|delete|patch))?"#,
    )
    .expect("swift alamofire regex");
    let re_urlrequest = regex::Regex::new(
        r#"URLRequest\s*\(\s*url\s*:\s*URL\s*\(\s*string\s*:\s*"(?P<url>[^"]+)""#,
    )
    .expect("swift urlrequest regex");

    let push = |out: &mut Vec<AbstractPoint>,
                key: String,
                line: u32,
                method: String,
                framework: &str| {
        let mut meta = HashMap::new();
        meta.insert("method".to_string(), method);
        meta.insert("framework".to_string(), framework.to_string());
        out.push(AbstractPoint {
            kind: ConnectionKind::Rest,
            role: ConnectionRole::Start,
            key,
            line,
            col: 1,
            symbol_qname: String::new(),
            meta,
        });
    };

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        // URLRequest wraps URL(string:) — check it first so URL() regex
        // doesn't double-emit for the same line.
        if let Some(cap) = re_urlrequest.captures(line_text) {
            let raw_url = cap["url"].to_string();
            if swift_rest_looks_like_api_url(&raw_url) {
                push(
                    out,
                    rest_normalise_url_pattern(&raw_url),
                    line_no,
                    "GET".to_string(),
                    "urlsession",
                );
                continue;
            }
        }

        if let Some(cap) = re_url.captures(line_text) {
            let raw_url = cap["url"].to_string();
            if swift_rest_looks_like_api_url(&raw_url) {
                push(
                    out,
                    rest_normalise_url_pattern(&raw_url),
                    line_no,
                    "GET".to_string(),
                    "urlsession",
                );
            }
        }

        if let Some(cap) = re_af.captures(line_text) {
            let raw_url = cap["url"].to_string();
            if swift_rest_looks_like_api_url(&raw_url) {
                let method = cap
                    .name("method")
                    .map(|m| m.as_str().to_uppercase())
                    .unwrap_or_else(|| "GET".to_string());
                push(
                    out,
                    rest_normalise_url_pattern(&raw_url),
                    line_no,
                    method,
                    "alamofire",
                );
            }
        }
    }
}

fn swift_rest_is_test_file(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    lower.contains("tests") || lower.contains("spec") || lower.contains("mock")
}

fn swift_rest_looks_like_api_url(s: &str) -> bool {
    if s.starts_with("http://") || s.starts_with("https://") {
        let after = s.find("://").map(|i| &s[i + 3..]).unwrap_or(s);
        let path = after.find('/').map(|i| &after[i..]).unwrap_or("");
        if path.is_empty() {
            return false;
        }
        return swift_rest_looks_like_api_url(path);
    }
    s.starts_with('/')
        || s.contains("/api/")
        || s.contains("/v1/")
        || s.contains("/v2/")
        || s.contains("/v3/")
        || s.contains("/{")
}

fn rest_normalise_url_pattern(raw: &str) -> String {
    raw.split('?').next().unwrap_or(raw).to_string()
}

#[cfg(test)]
mod plugin_source_scan_tests {
    use super::*;

    #[test]
    fn swift_rest_url_string_get() {
        let src = r#"let u = URL(string: "/api/users")!"#;
        let mut out = Vec::new();
        extract_swift_rest_starts_src(src, "Source/App.swift", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "/api/users");
        assert_eq!(out[0].meta.get("method").map(String::as_str), Some("GET"));
    }

    #[test]
    fn swift_rest_alamofire_method() {
        let src = r#"AF.request("/api/users", method: .post)"#;
        let mut out = Vec::new();
        extract_swift_rest_starts_src(src, "Source/App.swift", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].meta.get("method").map(String::as_str), Some("POST"));
        assert_eq!(out[0].meta.get("framework").map(String::as_str), Some("alamofire"));
    }

    #[test]
    fn swift_rest_skips_tests() {
        let src = r#"URL(string: "/api/x")"#;
        let mut out = Vec::new();
        extract_swift_rest_starts_src(src, "Tests/AppTests.swift", &mut out);
        assert!(out.is_empty());
    }
}
