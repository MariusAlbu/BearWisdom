// =============================================================================
// languages/graphql/connectors.rs — GraphQL schema Start-point detection
//
// Parses .graphql / .gql files for Query / Mutation / Subscription type blocks
// and emits Start connection points for each field.
//
// Flattened into the language plugin: `GraphQlPlugin::extract_connection_points`
// calls `extract_schema_starts` directly during parse (the plugin receives
// the source in memory, skipping the legacy disk read + DB roundtrip). The
// registry-facing `GraphQlSchemaConnector` impl returns empty — its work is
// now done at parse time.
//
// Stop points come from per-language consumer plugins (TypeScript via
// TypeScriptGraphQlConnector, Python via PythonGraphQlConnector, C# via
// CSharpGraphQlConnector) — those have not yet migrated to plugin-based
// emission.
// =============================================================================

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use regex::Regex;
use rusqlite::Connection;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint as DbPoint, Protocol};
use crate::indexer::project_context::ProjectContext;
use crate::types::{ConnectionKind, ConnectionPoint, ConnectionRole};

pub struct GraphQlSchemaConnector;

impl Connector for GraphQlSchemaConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "graphql_schema_starts",
            protocols: &[Protocol::GraphQl],
            languages: &["graphql"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        // Detection still fires so the registry knows the protocol is live;
        // the actual extraction path moved into the plugin.
        true
    }

    fn extract(&self, _conn: &Connection, _project_root: &Path) -> Result<Vec<DbPoint>> {
        // Extraction is now done at parse time via
        // `GraphQlPlugin::extract_connection_points`. The registry's
        // `run_with_plugin_points` path picks those up; this legacy method
        // returns empty so the point isn't emitted twice.
        Ok(Vec::new())
    }
}

/// Scan a `.graphql` / `.gql` source for top-level `type Query { … }`,
/// `type Mutation { … }`, `type Subscription { … }` blocks and emit a
/// Start `ConnectionPoint` per field. Called during parse from
/// `GraphQlPlugin::extract_connection_points`.
pub fn extract_schema_starts(source: &str) -> Vec<ConnectionPoint> {
    // Lazy statics would be nicer but regex compilation is still microsecond-
    // scale; this runs at parse time per file, not once per query.
    let re_type_block =
        Regex::new(r"type\s+(Query|Mutation|Subscription)\s*\{").expect("graphql type block regex");
    let re_field =
        Regex::new(r"^\s+(\w+)(?:\([^)]*\))?\s*:").expect("graphql field regex");

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

        if current_op_type.is_none() {
            continue;
        }

        for ch in line_text.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => {
                    if brace_depth > 0 {
                        brace_depth -= 1;
                    }
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
                if field_name.starts_with("__") {
                    continue;
                }
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

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_one_start_per_query_field() {
        let src = r#"
type Query {
  user(id: ID!): User
  users: [User!]!
}

type Mutation {
  createUser(input: UserInput!): User
}
"#;
        let points = extract_schema_starts(src);
        assert_eq!(points.len(), 3);
        assert_eq!(points[0].key, "user");
        assert_eq!(points[0].meta.get("method").map(String::as_str), Some("query"));
        assert_eq!(points[1].key, "users");
        assert_eq!(points[2].key, "createUser");
        assert_eq!(
            points[2].meta.get("method").map(String::as_str),
            Some("mutation")
        );
    }

    #[test]
    fn ignores_introspection_fields() {
        let src = "type Query {\n  __schema: Schema\n  real: String\n}\n";
        let points = extract_schema_starts(src);
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].key, "real");
    }

    #[test]
    fn empty_source_produces_no_points() {
        assert!(extract_schema_starts("").is_empty());
    }
}
