// =============================================================================
// languages/groovy/connectors.rs — Groovy-specific flow connectors
//
// GroovySpringRouteConnector:
//   Scans indexed Groovy files for Spring Web MVC route annotations
//   (@GetMapping, @PostMapping, @PutMapping, @DeleteMapping, @PatchMapping,
//   @RequestMapping) and emits REST Stop connection points.
//
// Flattened into `GroovyPlugin::extract_connection_points` at parse time.
// The legacy `Connector::extract` returns empty so the point isn't emitted
// twice. The symbol_id lookup that used to live in the DB path is dropped —
// the route's `symbol_qname` is empty, so the bridge won't try to map it.
// =============================================================================

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use regex::Regex;
use rusqlite::Connection;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, Protocol};
use crate::indexer::project_context::ProjectContext;
use crate::types::{
    ConnectionKind, ConnectionPoint as AbstractPoint, ConnectionRole,
};

// ===========================================================================
// Plugin-facing composer
// ===========================================================================

pub fn extract_groovy_connection_points(source: &str, _file_path: &str) -> Vec<AbstractPoint> {
    let mut out = Vec::new();
    extract_groovy_spring_routes_src(source, &mut out);
    out
}

// ===========================================================================
// GroovySpringRouteConnector — neutered (detect still gates; emission is at
// parse time via `extract_groovy_spring_routes_src`).
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

    fn extract(&self, _conn: &Connection, _project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        Ok(Vec::new())
    }
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

fn build_method_mapping_regex() -> Regex {
    Regex::new(
        r#"@(Get|Post|Put|Delete|Patch)Mapping\s*\(\s*(?:value\s*=\s*)?["']([^"']+)["']"#,
    )
    .expect("groovy method mapping regex")
}

fn build_request_mapping_regex() -> Regex {
    Regex::new(
        r#"@RequestMapping\s*\(\s*(?:value\s*=\s*)?["']([^"']+)["'](?:[^)]*method\s*=\s*RequestMethod\.(\w+))?"#,
    )
    .expect("groovy request mapping regex")
}

pub fn extract_groovy_spring_routes_src(source: &str, out: &mut Vec<AbstractPoint>) {
    if !source.contains("Mapping") && !source.contains("RequestMapping") {
        return;
    }

    let re_method = build_method_mapping_regex();
    let re_request = build_request_mapping_regex();

    let lines: Vec<&str> = source.lines().collect();
    let mut class_prefix = String::new();

    // First pass: find class-level @RequestMapping for the prefix.
    for (idx, line) in lines.iter().enumerate() {
        if let Some(cap) = re_request.captures(line) {
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

    for (idx, line_text) in lines.iter().enumerate() {
        let line_no = (idx + 1) as u32;

        if let Some(cap) = re_method.captures(line_text) {
            let verb = cap[1].to_uppercase();
            let path = format!("{}{}", class_prefix, &cap[2]);
            push(out, path, verb, line_no);
            continue;
        }

        if let Some(cap) = re_request.captures(line_text) {
            let is_class_level = lines[idx + 1..]
                .iter()
                .take(3)
                .any(|l| l.trim_start().starts_with("class "));
            if !is_class_level {
                let verb = cap
                    .get(2)
                    .map(|m| m.as_str().to_uppercase())
                    .unwrap_or_else(|| "GET".to_string());
                let path = format!("{}{}", class_prefix, &cap[1]);
                push(out, path, verb, line_no);
            }
        }
    }
}

fn push(out: &mut Vec<AbstractPoint>, key: String, method: String, line: u32) {
    let mut meta = HashMap::new();
    meta.insert("method".to_string(), method);
    meta.insert("framework".to_string(), "spring".to_string());
    out.push(AbstractPoint {
        kind: ConnectionKind::Rest,
        role: ConnectionRole::Stop,
        key,
        line,
        col: 1,
        symbol_qname: String::new(),
        meta,
    });
}

#[cfg(test)]
mod plugin_source_scan_tests {
    use super::*;

    #[test]
    fn groovy_get_mapping_emits_stop() {
        let src = r#"@GetMapping("/api/users")\ndef list() {}"#;
        let mut out = Vec::new();
        extract_groovy_spring_routes_src(src, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "/api/users");
        assert_eq!(out[0].meta.get("method").map(String::as_str), Some("GET"));
        assert_eq!(out[0].role, ConnectionRole::Stop);
    }

    #[test]
    fn groovy_class_level_request_mapping_prefixes_method_path() {
        let src = r#"@RequestMapping("/api")
class UsersController {
    @GetMapping("/users") def list() {}
}"#;
        let mut out = Vec::new();
        extract_groovy_spring_routes_src(src, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "/api/users");
    }
}
