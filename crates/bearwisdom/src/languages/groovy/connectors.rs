// =============================================================================
// languages/groovy/connectors.rs — Groovy-specific flow connectors
//
// GroovySpringRouteConnector:
//   Scans indexed Groovy files for Spring Web MVC route annotations
//   (@GetMapping, @PostMapping, @PutMapping, @DeleteMapping, @PatchMapping,
//   @RequestMapping) and emits REST Stop connection points.
//
//   Groovy is widely used with Spring (Grails, Micronaut) and supports the
//   same annotation syntax as Java.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::debug;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// GroovySpringRouteConnector
// ===========================================================================

pub struct GroovySpringRouteConnector;

impl Connector for GroovySpringRouteConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "groovy_spring_routes",
            protocols: &[Protocol::Rest],
            languages: &["groovy"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        true
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let routes = find_groovy_spring_routes(conn, project_root)
            .context("Groovy Spring route detection failed")?;

        Ok(routes
            .into_iter()
            .map(|r| ConnectionPoint {
                file_id: r.file_id,
                symbol_id: r.symbol_id,
                line: r.line,
                protocol: Protocol::Rest,
                direction: FlowDirection::Stop,
                key: r.path,
                method: r.http_method,
                framework: "spring".to_string(),
                metadata: None,
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Route record
// ---------------------------------------------------------------------------

struct GroovyRoute {
    file_id: i64,
    symbol_id: Option<i64>,
    http_method: String,
    path: String,
    line: u32,
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// @GetMapping/@PostMapping/etc. — captures (1) verb, (2) path.
fn build_method_mapping_regex() -> Regex {
    Regex::new(
        r#"@(Get|Post|Put|Delete|Patch)Mapping\s*\(\s*(?:value\s*=\s*)?["']([^"']+)["']"#,
    )
    .expect("groovy method mapping regex")
}

/// @RequestMapping with optional method= — captures (1) path, (2) optional verb.
fn build_request_mapping_regex() -> Regex {
    Regex::new(
        r#"@RequestMapping\s*\(\s*(?:value\s*=\s*)?["']([^"']+)["'](?:[^)]*method\s*=\s*RequestMethod\.(\w+))?"#,
    )
    .expect("groovy request mapping regex")
}

/// Method/closure name — captures (1) name.
fn build_method_name_regex() -> Regex {
    Regex::new(r"(?:def|public|protected|private)\s+(?:\w+\s+)?(\w+)\s*\(")
        .expect("groovy method name regex")
}

fn find_groovy_spring_routes(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<GroovyRoute>> {
    let re_method = build_method_mapping_regex();
    let re_request = build_request_mapping_regex();
    let re_method_name = build_method_name_regex();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'groovy'")
        .context("Failed to prepare Groovy files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Groovy files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Groovy file rows")?;

    let mut routes = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable Groovy file");
                continue;
            }
        };

        // Quick filter: skip files with no Spring mapping annotations.
        if !source.contains("Mapping") && !source.contains("RequestMapping") {
            continue;
        }

        extract_groovy_routes(
            conn,
            &source,
            file_id,
            &re_method,
            &re_request,
            &re_method_name,
            &mut routes,
        );
    }

    debug!(count = routes.len(), "Groovy Spring routes found");
    Ok(routes)
}

fn extract_groovy_routes(
    conn: &Connection,
    source: &str,
    file_id: i64,
    re_method: &Regex,
    re_request: &Regex,
    re_method_name: &Regex,
    out: &mut Vec<GroovyRoute>,
) {
    let lines: Vec<&str> = source.lines().collect();
    let mut class_prefix = String::new();

    // First pass: find class-level @RequestMapping for the prefix.
    for (idx, line) in lines.iter().enumerate() {
        if let Some(cap) = re_request.captures(line) {
            // Check if the next non-blank line is a class declaration.
            let is_class_level = lines[idx + 1..]
                .iter()
                .take(3)
                .any(|l| l.trim_start().starts_with("class "));
            if is_class_level {
                class_prefix = cap[1].to_string();
                break;
            }
        }
    }

    // Second pass: extract method-level mappings.
    let mut pending_annotation: Option<(String, String, u32)> = None; // (method, path, line)

    for (idx, line_text) in lines.iter().enumerate() {
        let line_no = (idx + 1) as u32;

        if let Some(cap) = re_method.captures(line_text) {
            let verb = cap[1].to_uppercase();
            let path = format!("{}{}", class_prefix, &cap[2]);
            pending_annotation = Some((verb, path, line_no));
            continue;
        }

        if let Some(cap) = re_request.captures(line_text) {
            let is_class_level = lines[idx + 1..]
                .iter()
                .take(3)
                .any(|l| l.trim_start().starts_with("class "));
            if !is_class_level {
                let verb = cap.get(2).map(|m| m.as_str().to_uppercase()).unwrap_or_else(|| "GET".to_string());
                let path = format!("{}{}", class_prefix, &cap[1]);
                pending_annotation = Some((verb, path, line_no));
            }
            continue;
        }

        if let Some((verb, path, ann_line)) = pending_annotation.take() {
            // Try to find a method name on this or the next line.
            let handler_name = re_method_name
                .captures(line_text)
                .map(|c| c[1].to_string())
                .unwrap_or_default();

            let symbol_id: Option<i64> = if !handler_name.is_empty() {
                conn.query_row(
                    "SELECT id FROM symbols WHERE file_id = ?1 AND name = ?2 LIMIT 1",
                    rusqlite::params![file_id, handler_name],
                    |r| r.get(0),
                )
                .ok()
            } else {
                None
            };

            out.push(GroovyRoute {
                file_id,
                symbol_id,
                http_method: verb,
                path,
                line: ann_line,
            });
        }
    }
}
