// =============================================================================
// connectors/registry.rs — ConnectorRegistry
//
// Lifecycle:
//   1. detect  — filter connectors to those relevant for the current project
//   2. extract — collect ConnectionPoints from each active connector
//   3. match   — group by protocol, run ProtocolMatcher (or custom_match),
//                write ResolvedFlows as flow_edges rows
//   4. compat  — back-fill the legacy `routes` table from REST stops
// =============================================================================

use anyhow::Result;
use rustc_hash::FxHashSet;
use rusqlite::Connection;
use std::path::Path;
use tracing::{info, warn};

use super::connector_db::{
    clear_connection_points, populate_routes_from_stops, store_connection_points, write_flow_edges,
};
use super::matcher::ProtocolMatcher;
use super::traits::Connector;
use super::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

pub struct ConnectorRegistry {
    connectors: Vec<Box<dyn Connector>>,
}

impl ConnectorRegistry {
    /// Create an empty registry.
    ///
    /// Connectors will be registered here as they are migrated to the new
    /// architecture.  The existing connector modules continue to run through
    /// the old `full.rs` pipeline in parallel during the migration.
    pub fn new() -> Self {
        Self {
            connectors: vec![],
        }
    }

    /// Register a connector.
    pub fn register(&mut self, connector: Box<dyn Connector>) {
        self.connectors.push(connector);
    }

    /// Run the full extract → match → write pipeline.
    ///
    /// Returns the total number of flow_edges rows written.
    pub fn run(
        &self,
        conn: &Connection,
        project_root: &Path,
        ctx: &ProjectContext,
    ) -> Result<u32> {
        // Phase 0: detect — only keep connectors relevant to this project.
        let active: Vec<&dyn Connector> = self
            .connectors
            .iter()
            .filter(|c| c.detect(ctx))
            .map(|c| c.as_ref())
            .collect();

        if active.is_empty() {
            return Ok(0);
        }

        // Phase 1: extract — gather all ConnectionPoints.
        clear_connection_points(conn)?;

        let mut all_points: Vec<ConnectionPoint> = Vec::new();

        for connector in &active {
            let desc = connector.descriptor();
            match connector.extract(conn, project_root) {
                Ok(points) => {
                    info!("{}: {} connection points", desc.name, points.len());
                    all_points.extend(points);
                }
                Err(e) => {
                    warn!("{}: extraction failed: {e}", desc.name);
                }
            }
        }

        // Dedup points by (file_id, line, protocol, direction, key, method)
        // before matching — multiple connectors may emit the same stop.
        dedup_points(&mut all_points);

        store_connection_points(conn, &all_points)?;

        // Phase 2: match — group by protocol, run matching, write edges.
        let protocols: Vec<Protocol> = unique_protocols(&all_points);

        let mut total = 0u32;

        for protocol in protocols {
            let starts: Vec<&ConnectionPoint> = all_points
                .iter()
                .filter(|cp| cp.protocol == protocol && cp.direction == FlowDirection::Start)
                .collect();

            let stops: Vec<&ConnectionPoint> = all_points
                .iter()
                .filter(|cp| cp.protocol == protocol && cp.direction == FlowDirection::Stop)
                .collect();

            if starts.is_empty() || stops.is_empty() {
                continue;
            }

            // Give the first connector that covers this protocol a chance to
            // provide a custom matching strategy.
            let mut custom: Option<Vec<_>> = None;
            for c in &active {
                if c.descriptor().protocols.contains(&protocol) {
                    // Pass owned vecs (cloned) because custom_match takes slices.
                    let owned_starts: Vec<ConnectionPoint> =
                        starts.iter().map(|cp| (*cp).clone()).collect();
                    let owned_stops: Vec<ConnectionPoint> =
                        stops.iter().map(|cp| (*cp).clone()).collect();

                    if let Ok(Some(flows)) = c.custom_match(conn, &owned_starts, &owned_stops) {
                        custom = Some(flows);
                        break;
                    }
                }
            }

            let flows = match custom {
                Some(f) => f,
                None => ProtocolMatcher::match_protocol(protocol, &starts, &stops),
            };

            let n = write_flow_edges(conn, &flows)?;
            info!(
                "{}: {} starts × {} stops → {} edges",
                protocol.as_str(),
                starts.len(),
                stops.len(),
                n
            );
            total += n;
        }

        // Phase 3: back-fill legacy routes table.
        let route_rows = populate_routes_from_stops(conn)?;
        if route_rows > 0 {
            info!("Populated {} legacy routes from REST stops", route_rows);
        }

        Ok(total)
    }
}

impl Default for ConnectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a registry pre-loaded with all built-in connectors.
pub fn build_default_registry() -> ConnectorRegistry {
    let mut reg = ConnectorRegistry::new();
    reg.register(Box::new(super::rest_connector::RestConnector));
    reg.register(Box::new(super::grpc_connector::GrpcConnector));
    reg.register(Box::new(super::mq_connector::MessageQueueConnector));
    reg.register(Box::new(super::graphql_connector::GraphQlConnector));
    reg.register(Box::new(super::event_connector::EventBusConnector));
    reg.register(Box::new(super::ipc_connector::TauriIpcConnector));
    reg.register(Box::new(super::ipc_connector::ElectronIpcConnector));
    // Framework-specific route producers
    reg.register(Box::new(super::route_connectors::SpringRouteConnector));
    reg.register(Box::new(super::route_connectors::DjangoRouteConnector));
    reg.register(Box::new(super::route_connectors::FastApiRouteConnector));
    reg.register(Box::new(super::route_connectors::GoRouteConnector));
    reg.register(Box::new(super::route_connectors::RailsRouteConnector));
    reg.register(Box::new(super::route_connectors::LaravelRouteConnector));
    reg.register(Box::new(super::route_connectors::NestjsRouteConnector));
    reg.register(Box::new(super::route_connectors::NextjsRouteConnector));
    // DI connectors
    reg.register(Box::new(super::di_connector::DotnetDiConnector));
    reg.register(Box::new(super::di_connector::AngularDiConnector));
    reg.register(Box::new(super::di_connector::SpringDiConnector));
    reg
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn unique_protocols(points: &[ConnectionPoint]) -> Vec<Protocol> {
    let mut seen = FxHashSet::default();
    let mut protocols = Vec::new();
    for cp in points {
        if seen.insert(cp.protocol) {
            protocols.push(cp.protocol);
        }
    }
    protocols
}

/// Remove duplicate connection points in-place.
///
/// Key: (file_id, line, protocol, direction, key, method).  When two connectors
/// emit the same stop (e.g. RestConnector from `routes` table + SpringRouteConnector
/// from regex), the first one wins.
fn dedup_points(points: &mut Vec<ConnectionPoint>) {
    let mut seen = FxHashSet::default();
    points.retain(|cp| {
        seen.insert((
            cp.file_id,
            cp.line,
            cp.protocol,
            cp.direction,
            cp.key.clone(),
            cp.method.clone(),
        ))
    });
}
