// =============================================================================
// languages/dart/connectors.rs — Dart REST connector
//
// DartRestConnector:
//   Start points: http package (http.get/post/…), Dio (dio.get/post/…),
//                 Chopper annotations (@Get/@Post/…).
//   Stop points:  Route handler registrations in the `routes` table for Dart.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// DartRestConnector
// ===========================================================================

pub struct DartRestConnector;

impl Connector for DartRestConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "dart_rest",
            protocols: &[Protocol::Rest],
            languages: &["dart"],
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
        extract_dart_rest_stops(conn, &mut points)?;
        extract_dart_rest_starts(conn, project_root, &mut points)?;
        Ok(points)
    }
}

// ---------------------------------------------------------------------------
// Stop extraction
// ---------------------------------------------------------------------------

fn extract_dart_rest_stops(conn: &Connection, out: &mut Vec<ConnectionPoint>) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT r.file_id, r.symbol_id, r.line, r.http_method,
                    COALESCE(r.resolved_route, r.route_template)
             FROM routes r
             JOIN files f ON f.id = r.file_id
             WHERE f.language = 'dart'
               AND r.http_method != '' AND r.route_template != ''",
        )
        .context("Failed to prepare Dart REST stops query")?;

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
        .context("Failed to query Dart routes")?;

    for row in rows {
        let (file_id, symbol_id, line, method, route) =
            row.context("Failed to read Dart route row")?;
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

fn extract_dart_rest_starts(
    conn: &Connection,
    project_root: &Path,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    // http package: http.get(Uri.parse("url")), http.post(Uri.parse("url"))
    let re_http_pkg = regex::Regex::new(
        r#"http\s*\.\s*(?P<method>get|post|put|delete|patch|head)\s*\(\s*Uri\.parse\s*\(\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)')"#,
    )
    .expect("dart http pkg regex");

    // Dio: dio.get("url"), dio.post("url"), _dio.get("url")
    let re_dio = regex::Regex::new(
        r#"(?:dio|_dio|client)\s*\.\s*(?P<method>get|post|put|delete|patch|head)\s*(?:<[^>]*>)?\s*\(\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)')"#,
    )
    .expect("dart dio regex");

    // Chopper: @Get(path: "/api/…"), @Post(path: "/api/…")
    let re_chopper = regex::Regex::new(
        r#"@(?P<method>Get|Post|Put|Delete|Patch)\s*\(\s*path\s*:\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)')"#,
    )
    .expect("dart chopper annotation regex");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'dart'")
        .context("Failed to prepare Dart files query")?;
    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Dart files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Dart file rows")?;

    for (file_id, rel_path) in files {
        if dart_rest_is_test_file(&rel_path) {
            continue;
        }
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;

            for (re, framework) in &[(&re_http_pkg, "http"), (&re_dio, "dio")] {
                for cap in re.captures_iter(line_text) {
                    let raw_url = cap
                        .name("url1")
                        .or_else(|| cap.name("url2"))
                        .map(|m| m.as_str().to_string());
                    let Some(raw_url) = raw_url else { continue };
                    if !dart_rest_looks_like_api_url(&raw_url) {
                        continue;
                    }
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
                        framework: framework.to_string(),
                        metadata: None,
                    });
                }
            }

            // Chopper annotations
            for cap in re_chopper.captures_iter(line_text) {
                let raw_url = cap
                    .name("url1")
                    .or_else(|| cap.name("url2"))
                    .map(|m| m.as_str().to_string());
                let Some(raw_url) = raw_url else { continue };
                if !dart_rest_looks_like_api_url(&raw_url) {
                    continue;
                }
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
                    framework: "chopper".to_string(),
                    metadata: None,
                });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn dart_rest_is_test_file(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    lower.contains("_test.dart") || lower.contains("/test/") || lower.contains("/tests/")
}

fn dart_rest_looks_like_api_url(s: &str) -> bool {
    if s.starts_with("http://") || s.starts_with("https://") {
        let after = s.find("://").map(|i| &s[i + 3..]).unwrap_or(s);
        let path = after.find('/').map(|i| &after[i..]).unwrap_or("");
        if path.is_empty() {
            return false;
        }
        return dart_rest_looks_like_api_url(path);
    }
    s.starts_with('/')
        || s.contains("/api/")
        || s.contains("/v1/")
        || s.contains("/v2/")
        || s.contains("/v3/")
        || s.contains("/{")
}

fn rest_normalise_url_pattern(raw: &str) -> String {
    let without_query = raw.split('?').next().unwrap_or(raw);
    let re_tmpl = regex::Regex::new(r"\$\{[^}]+\}").expect("template regex");
    re_tmpl.replace_all(without_query, "{param}").into_owned()
}
