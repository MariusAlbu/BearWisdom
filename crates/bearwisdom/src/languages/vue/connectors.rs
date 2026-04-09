// =============================================================================
// languages/vue/connectors.rs — Vue-specific flow connectors
//
// VueGraphQlConnector:
//   Detects GraphQL operations in Vue Single File Components (.vue files).
//   Vue apps use Apollo Client (@vue/apollo-composable) or URQL.
//
//   Start points: gql`` template literals containing type Query/Mutation/Subscription.
//   Stop points: resolver map entries (Nuxt server routes with Apollo Server).
//
//   Detection: ts_packages contains "graphql", "@apollo/client",
//              "@vue/apollo-composable", or "@urql/vue".
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::debug;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// VueGraphQlConnector
// ===========================================================================

pub struct VueGraphQlConnector;

impl Connector for VueGraphQlConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "vue_graphql",
            protocols: &[Protocol::GraphQl],
            languages: &["vue"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.has_dependency(ManifestKind::Npm, "graphql")
            || ctx.has_dependency(ManifestKind::Npm, "@apollo/client")
            || ctx.has_dependency(ManifestKind::Npm, "apollo-client")
            || ctx.has_dependency(ManifestKind::Npm, "@vue/apollo-composable")
            || ctx.has_dependency(ManifestKind::Npm, "vue-apollo")
            || ctx.has_dependency(ManifestKind::Npm, "urql")
            || ctx.has_dependency(ManifestKind::Npm, "@urql/vue")
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let re_type_block = Regex::new(r"type\s+(Query|Mutation|Subscription)\s*\{")
            .expect("vue gql type block regex");
        let re_field = Regex::new(r"^\s+(\w+)(?:\([^)]*\))?\s*:")
            .expect("vue gql field regex");
        let re_resolver_key = Regex::new(
            r#"['"`]?(\w+)['"`]?\s*:\s*(?:async\s+)?\([^)]*\)\s*=>"#,
        )
        .expect("vue graphql resolver key regex");

        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE language = 'vue'")
            .context("Failed to prepare Vue files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .context("Failed to query Vue files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Vue file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            let lower = rel_path.to_lowercase();
            if lower.contains("/node_modules/") {
                continue;
            }

            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(e) => {
                    debug!(path = %abs_path.display(), err = %e, "Skipping unreadable Vue file");
                    continue;
                }
            };

            if !re_type_block.is_match(&source) && !source.contains("gql`") {
                continue;
            }

            extract_graphql_points(
                &source,
                file_id,
                &re_type_block,
                &re_field,
                &re_resolver_key,
                &mut points,
            );
        }

        Ok(points)
    }
}

fn extract_graphql_points(
    source: &str,
    file_id: i64,
    re_type_block: &Regex,
    re_field: &Regex,
    re_resolver_key: &Regex,
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

        if current_op_type.is_some() {
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
                    if !field_name.starts_with("__") {
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
            continue;
        }

        // Resolver map entries (Nuxt server API routes with Apollo/URQL).
        for cap in re_resolver_key.captures_iter(line_text) {
            let name = cap[1].to_string();
            if matches!(
                name.as_str(),
                "then" | "catch" | "finally" | "map" | "filter" | "reduce"
            ) {
                continue;
            }
            out.push(ConnectionPoint {
                file_id,
                symbol_id: None,
                line: line_no,
                protocol: Protocol::GraphQl,
                direction: FlowDirection::Stop,
                key: name,
                method: String::new(),
                framework: "apollo".to_string(),
                metadata: None,
            });
        }
    }
}
