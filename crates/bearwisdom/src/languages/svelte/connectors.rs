// =============================================================================
// languages/svelte/connectors.rs — Svelte GraphQL connection points
//
// Flattened into `SveltePlugin::extract_connection_points` — the scan runs at
// parse time on the in-memory source. The registry-facing `Connector` impl
// returns empty; its detect still fires so the protocol is considered live.
// =============================================================================

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use regex::Regex;
use rusqlite::Connection;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint as DbPoint, Protocol};
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::types::{ConnectionKind, ConnectionPoint, ConnectionRole};

pub struct SvelteGraphQlConnector;

impl Connector for SvelteGraphQlConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "svelte_graphql",
            protocols: &[Protocol::GraphQl],
            languages: &["svelte"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.has_dependency(ManifestKind::Npm, "graphql")
            || ctx.has_dependency(ManifestKind::Npm, "@apollo/client")
            || ctx.has_dependency(ManifestKind::Npm, "apollo-client")
            || ctx.has_dependency(ManifestKind::Npm, "urql")
            || ctx.has_dependency(ManifestKind::Npm, "@urql/svelte")
    }

    fn extract(&self, _conn: &Connection, _project_root: &Path) -> Result<Vec<DbPoint>> {
        // Moved into `SveltePlugin::extract_connection_points`.
        Ok(Vec::new())
    }
}

/// Scan a `.svelte` source for embedded GraphQL schema definitions and
/// resolver-map entries. Called during parse from
/// `SveltePlugin::extract_connection_points`.
pub fn extract_svelte_graphql_points(source: &str) -> Vec<ConnectionPoint> {
    let re_type_block =
        Regex::new(r"type\s+(Query|Mutation|Subscription)\s*\{").expect("svelte gql type block regex");
    let re_field =
        Regex::new(r"^\s+(\w+)(?:\([^)]*\))?\s*:").expect("svelte gql field regex");
    let re_resolver_key =
        Regex::new(r#"['"`]?(\w+)['"`]?\s*:\s*(?:async\s+)?\([^)]*\)\s*=>"#)
            .expect("svelte graphql resolver key regex");

    // Fast skip for files that contain neither a GraphQL schema block nor a
    // `gql` template literal — avoids line-by-line scanning on every `.svelte`.
    if !re_type_block.is_match(source) && !source.contains("gql`") {
        return Vec::new();
    }

    let mut out: Vec<ConnectionPoint> = Vec::new();
    let mut current_op_type: Option<String> = None;
    let mut brace_depth: u32 = 0;

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        if let Some(cap) = re_type_block.captures(line_text) {
            current_op_type = Some(cap[1].to_lowercase());
            brace_depth = 1;
            continue;
        }

        if current_op_type.is_some() {
            for ch in line_text.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => {
                        if brace_depth > 0 { brace_depth -= 1; }
                    }
                    _ => {}
                }
            }
            if brace_depth == 0 {
                current_op_type = None;
                continue;
            }
            if brace_depth == 1 {
                if let Some(cap) = re_field.captures(line_text) {
                    let field_name = cap[1].to_string();
                    if !field_name.starts_with("__") {
                        let mut meta = HashMap::new();
                        if let Some(op) = current_op_type.as_deref() {
                            meta.insert("method".to_string(), op.to_string());
                        }
                        out.push(ConnectionPoint {
                            kind: ConnectionKind::GraphQL,
                            role: ConnectionRole::Start,
                            key: field_name,
                            line: line_no,
                            col: 1,
                            symbol_qname: String::new(),
                            meta,
                        });
                    }
                }
            }
            continue;
        }

        // Resolver map entries (SvelteKit API routes with Apollo Server).
        for cap in re_resolver_key.captures_iter(line_text) {
            let name = cap[1].to_string();
            if matches!(
                name.as_str(),
                "then" | "catch" | "finally" | "map" | "filter" | "reduce"
            ) {
                continue;
            }
            let mut meta = HashMap::new();
            meta.insert("framework".to_string(), "apollo".to_string());
            out.push(ConnectionPoint {
                kind: ConnectionKind::GraphQL,
                role: ConnectionRole::Stop,
                key: name,
                line: line_no,
                col: 1,
                symbol_qname: String::new(),
                meta,
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_empty_without_gql_markers() {
        assert!(extract_svelte_graphql_points("<script>let x = 1;</script>").is_empty());
    }

    #[test]
    fn emits_schema_starts_from_type_blocks() {
        let src = "type Query {\n  me: User\n}\n";
        let points = extract_svelte_graphql_points(src);
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].key, "me");
        assert_eq!(points[0].role, ConnectionRole::Start);
    }

    #[test]
    fn emits_resolver_stops() {
        let src = "gql`\nconst resolvers = {\n  myField: async (a, b) => null,\n};\n";
        let points = extract_svelte_graphql_points(src);
        assert!(
            points.iter().any(|p| p.key == "myField" && p.role == ConnectionRole::Stop),
            "expected myField stop, got {points:?}",
        );
    }
}
