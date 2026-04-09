// =============================================================================
// languages/swift/connectors.rs — Swift REST connector
//
// SwiftRestConnector:
//   Start points: URLSession (URL(string:)), Alamofire (AF.request),
//                 URLRequest(url: URL(string:)).
//   Stop points:  Route handler registrations in the `routes` table for Swift.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// SwiftRestConnector
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
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let mut points = Vec::new();
        extract_swift_rest_stops(conn, &mut points)?;
        extract_swift_rest_starts(conn, project_root, &mut points)?;
        Ok(points)
    }
}

// ---------------------------------------------------------------------------
// Stop extraction
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Start extraction
// ---------------------------------------------------------------------------

fn extract_swift_rest_starts(
    conn: &Connection,
    project_root: &Path,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    // URLSession: URL(string: "url")
    let re_url = regex::Regex::new(r#"URL\s*\(\s*string\s*:\s*"(?P<url>[^"]+)""#)
        .expect("swift url regex");

    // Alamofire: AF.request("url", method: .get/.post/…)
    let re_af = regex::Regex::new(
        r#"AF\s*\.\s*request\s*\(\s*"(?P<url>[^"]+)"(?:\s*,\s*method\s*:\s*\.(?P<method>get|post|put|delete|patch))?"#,
    )
    .expect("swift alamofire regex");

    // URLRequest: URLRequest(url: URL(string: "url")!)
    let re_urlrequest = regex::Regex::new(
        r#"URLRequest\s*\(\s*url\s*:\s*URL\s*\(\s*string\s*:\s*"(?P<url>[^"]+)""#,
    )
    .expect("swift urlrequest regex");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'swift'")
        .context("Failed to prepare Swift files query")?;
    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Swift files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Swift file rows")?;

    for (file_id, rel_path) in files {
        if swift_rest_is_test_file(&rel_path) {
            continue;
        }
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;

            // URL(string: "url") — URLSession-style
            if let Some(cap) = re_url.captures(line_text) {
                let raw_url = cap["url"].to_string();
                if swift_rest_looks_like_api_url(&raw_url) {
                    let url_pattern = rest_normalise_url_pattern(&raw_url);
                    out.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::Rest,
                        direction: FlowDirection::Start,
                        key: url_pattern,
                        method: "GET".to_string(),
                        framework: "urlsession".to_string(),
                        metadata: None,
                    });
                }
            }

            // AF.request("url", method: .post)
            if let Some(cap) = re_af.captures(line_text) {
                let raw_url = cap["url"].to_string();
                if swift_rest_looks_like_api_url(&raw_url) {
                    let method = cap
                        .name("method")
                        .map(|m| m.as_str().to_uppercase())
                        .unwrap_or_else(|| "GET".to_string());
                    let url_pattern = rest_normalise_url_pattern(&raw_url);
                    out.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::Rest,
                        direction: FlowDirection::Start,
                        key: url_pattern,
                        method,
                        framework: "alamofire".to_string(),
                        metadata: None,
                    });
                }
            }

            // URLRequest(url: URL(string: "url")!)
            if let Some(cap) = re_urlrequest.captures(line_text) {
                let raw_url = cap["url"].to_string();
                if swift_rest_looks_like_api_url(&raw_url) {
                    let url_pattern = rest_normalise_url_pattern(&raw_url);
                    out.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::Rest,
                        direction: FlowDirection::Start,
                        key: url_pattern,
                        method: "GET".to_string(),
                        framework: "urlsession".to_string(),
                        metadata: None,
                    });
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
