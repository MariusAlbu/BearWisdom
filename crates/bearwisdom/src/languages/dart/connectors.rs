// =============================================================================
// languages/dart/connectors.rs — Dart REST connector
//
// Start points (source-scan): http package, Dio, Chopper annotations.
// Stop points: route handlers in the `routes` table.
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

pub fn extract_dart_connection_points(source: &str, file_path: &str) -> Vec<AbstractPoint> {
    let mut out = Vec::new();
    extract_dart_rest_starts_src(source, file_path, &mut out);
    out
}

// ===========================================================================
// DartRestConnector — starts flattened; stops on DB
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
        _project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let mut points = Vec::new();
        extract_dart_rest_stops(conn, &mut points)?;
        Ok(points)
    }
}

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

/// Dart REST client-call starts: http package, Dio, Chopper annotations.
pub fn extract_dart_rest_starts_src(
    source: &str,
    file_path: &str,
    out: &mut Vec<AbstractPoint>,
) {
    if dart_rest_is_test_file(file_path) {
        return;
    }
    if !source.contains("http.")
        && !source.contains("dio.")
        && !source.contains("_dio.")
        && !source.contains("client.")
        && !source.contains("@Get(")
        && !source.contains("@Post(")
        && !source.contains("@Put(")
        && !source.contains("@Delete(")
        && !source.contains("@Patch(")
    {
        return;
    }

    let re_http_pkg = regex::Regex::new(
        r#"http\s*\.\s*(?P<method>get|post|put|delete|patch|head)\s*\(\s*Uri\.parse\s*\(\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)')"#,
    )
    .expect("dart http pkg regex");
    let re_dio = regex::Regex::new(
        r#"(?:dio|_dio|client)\s*\.\s*(?P<method>get|post|put|delete|patch|head)\s*(?:<[^>]*>)?\s*\(\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)')"#,
    )
    .expect("dart dio regex");
    let re_chopper = regex::Regex::new(
        r#"@(?P<method>Get|Post|Put|Delete|Patch)\s*\(\s*path\s*:\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)')"#,
    )
    .expect("dart chopper annotation regex");

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
                push(out, rest_normalise_url_pattern(&raw_url), line_no, method, framework);
            }
        }

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
            push(out, rest_normalise_url_pattern(&raw_url), line_no, method, "chopper");
        }
    }
}

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

#[cfg(test)]
mod plugin_source_scan_tests {
    use super::*;

    #[test]
    fn dart_rest_dio_get() {
        let src = r#"await dio.get("/api/users")"#;
        let mut out = Vec::new();
        extract_dart_rest_starts_src(src, "lib/client.dart", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].meta.get("framework").map(String::as_str), Some("dio"));
    }

    #[test]
    fn dart_rest_chopper_post_annotation() {
        let src = r#"@Post(path: "/api/users") Future<User> create();"#;
        let mut out = Vec::new();
        extract_dart_rest_starts_src(src, "lib/api.dart", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].meta.get("method").map(String::as_str), Some("POST"));
        assert_eq!(out[0].meta.get("framework").map(String::as_str), Some("chopper"));
    }

    #[test]
    fn dart_rest_skips_tests() {
        let src = r#"dio.get("/api/x")"#;
        let mut out = Vec::new();
        extract_dart_rest_starts_src(src, "test/client_test.dart", &mut out);
        assert!(out.is_empty());
    }
}
