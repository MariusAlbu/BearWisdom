// =============================================================================
// languages/graphql/connectors.rs — GraphQL schema Start-point connector
//
// Parses .graphql / .gql files for Query / Mutation / Subscription type blocks
// and emits Start connection points for each field.
//
// Stop points come from the per-language connectors in the consuming languages
// (TypeScript via TypeScriptGraphQlConnector, Python via PythonGraphQlConnector,
// C# via CSharpGraphQlConnector).
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

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
        true
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let re_type_block = Regex::new(r"type\s+(Query|Mutation|Subscription)\s*\{")
            .expect("graphql type block regex");
        let re_field = Regex::new(r"^\s+(\w+)(?:\([^)]*\))?\s*:")
            .expect("graphql field regex");

        let mut stmt = conn
            .prepare(
                "SELECT id, path FROM files
                 WHERE language = 'graphql' OR path LIKE '%.graphql' OR path LIKE '%.gql'",
            )
            .context("Failed to prepare GraphQL files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .context("Failed to query GraphQL files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect GraphQL file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            extract_schema_starts(
                &source,
                file_id,
                &re_type_block,
                &re_field,
                &mut points,
            );
        }

        Ok(points)
    }
}

fn extract_schema_starts(
    source: &str,
    file_id: i64,
    re_type_block: &Regex,
    re_field: &Regex,
    out: &mut Vec<ConnectionPoint>,
) {
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
                out.push(ConnectionPoint {
                    file_id,
                    symbol_id: None,
                    line: line_no,
                    protocol: Protocol::GraphQl,
                    direction: FlowDirection::Start,
                    key: field_name,
                    method: current_op_type.clone().unwrap_or_default(),
                    framework: String::new(),
                    metadata: None,
                });
            }
        }
    }
}
