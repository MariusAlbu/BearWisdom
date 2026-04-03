// =============================================================================
// connectors/graphql_connector.rs — GraphQL connector (new architecture)
//
// Wraps the existing graphql.rs logic.
//
// Start points: GraphQL operations (query/mutation/subscription fields in
//               schema files or embedded SDL).
// Stop points: resolver functions/methods matching operation names.
//
// Custom matching: yes — the existing resolver matching is name-based against
// the symbols table, not just key equality.  We implement custom_match to
// preserve that logic.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::graphql;
use super::traits::{Connector, ConnectorDescriptor};
use super::types::{ConnectionPoint, FlowDirection, Protocol, ResolvedFlow};
use crate::indexer::project_context::ProjectContext;

pub struct GraphQlConnector;

impl Connector for GraphQlConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "graphql",
            protocols: &[Protocol::GraphQl],
            languages: &[
                "graphql", "typescript", "tsx", "javascript", "jsx", "python",
            ],
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
        let operations = graphql::detect_graphql_operations(conn, project_root)
            .context("GraphQL operation detection failed")?;

        let points = operations
            .into_iter()
            .map(|op| ConnectionPoint {
                file_id: op.file_id,
                symbol_id: None,
                line: op.line,
                protocol: Protocol::GraphQl,
                direction: FlowDirection::Start,
                key: op.name,
                method: op.operation_type,
                framework: String::new(),
                metadata: None,
            })
            .collect();

        Ok(points)
    }

    fn custom_match(
        &self,
        conn: &Connection,
        starts: &[ConnectionPoint],
        _stops: &[ConnectionPoint],
    ) -> Result<Option<Vec<ResolvedFlow>>> {
        // Custom matching: for each start (operation), find resolver symbols
        // by name in the symbols table and emit flows.
        let mut flows = Vec::new();

        for start in starts {
            let resolvers = find_resolvers(conn, &start.key)?;

            for (resolver_file_id, resolver_sym_id, resolver_line) in resolvers {
                let stop = ConnectionPoint {
                    file_id: resolver_file_id,
                    symbol_id: Some(resolver_sym_id),
                    line: resolver_line,
                    protocol: Protocol::GraphQl,
                    direction: FlowDirection::Stop,
                    key: start.key.clone(),
                    method: String::new(),
                    framework: String::new(),
                    metadata: None,
                };

                flows.push(ResolvedFlow {
                    start: start.clone(),
                    stop,
                    confidence: 0.80,
                    edge_type: "graphql_resolver".to_string(),
                });
            }
        }

        Ok(Some(flows))
    }
}

fn find_resolvers(
    conn: &Connection,
    operation_name: &str,
) -> Result<Vec<(i64, i64, u32)>> {
    let mut stmt = conn
        .prepare(
            "SELECT s.file_id, s.id, s.line FROM symbols s
             WHERE s.name = ?1 AND s.kind IN ('function', 'method')
             LIMIT 10",
        )
        .context("Failed to prepare resolver query")?;

    let rows = stmt
        .query_map(rusqlite::params![operation_name], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, u32>(2)?,
            ))
        })
        .context("Failed to query resolvers")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect resolver rows")?;

    Ok(rows)
}
