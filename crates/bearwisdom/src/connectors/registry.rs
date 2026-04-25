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
use std::collections::HashSet;
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
    /// All flow-based connectors (REST, gRPC, MQ, GraphQL, events, IPC, DI,
    /// routes) are registered here.  Non-flow post-index hooks (EF Core DB
    /// mappings, Django model linking, React concept creation) run separately
    /// in `full.rs` because they write to different tables (db_mappings,
    /// concepts), not flow_edges.
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
        self.run_with_plugin_points(conn, project_root, ctx, &[], &[])
    }

    /// Variant of `run` that accepts a pre-collected list of
    /// `ConnectionPoint`s emitted by language plugins during parse
    /// (via `LanguagePlugin::extract_connection_points` → converted by
    /// `connectors::from_plugins::collect_plugin_connection_points`). The
    /// pre-collected points are dumped into the same pool as the ones
    /// produced by legacy `Connector::extract` impls; dedup removes
    /// accidental duplicates when a connector hasn't yet been flattened
    /// but its plugin also emits some of the same points.
    ///
    /// As individual connectors migrate their detection into plugin
    /// `extract_connection_points`, their legacy `Connector::extract`
    /// shrinks / vanishes; this variant keeps both sources live during
    /// the migration.
    /// Incremental variant of `run_with_plugin_points` that scopes each
    /// connector's source-file scan to `changed_paths`. Used by the
    /// incremental save path so a 10k-file C# project doesn't pay the
    /// cost of re-reading every .cs file on every save just to detect
    /// DI registrations and event handlers in the 2-3 changed files.
    pub fn run_incremental(
        &self,
        conn: &Connection,
        project_root: &Path,
        ctx: &ProjectContext,
        plugin_points: &[ConnectionPoint],
        resolved_plugin_points: &[ConnectionPoint],
        changed_paths: &HashSet<String>,
    ) -> Result<u32> {
        self.run_inner(
            conn, project_root, ctx, plugin_points, resolved_plugin_points,
            Some(changed_paths),
        )
    }

    pub fn run_with_plugin_points(
        &self,
        conn: &Connection,
        project_root: &Path,
        ctx: &ProjectContext,
        plugin_points: &[ConnectionPoint],
        resolved_plugin_points: &[ConnectionPoint],
    ) -> Result<u32> {
        self.run_inner(conn, project_root, ctx, plugin_points, resolved_plugin_points, None)
    }

    fn run_inner(
        &self,
        conn: &Connection,
        project_root: &Path,
        ctx: &ProjectContext,
        plugin_points: &[ConnectionPoint],
        resolved_plugin_points: &[ConnectionPoint],
        changed_paths: Option<&HashSet<String>>,
    ) -> Result<u32> {
        // Phase 0: detect — only keep connectors relevant to this project.
        let active: Vec<&dyn Connector> = self
            .connectors
            .iter()
            .filter(|c| c.detect(ctx))
            .map(|c| c.as_ref())
            .collect();

        if active.is_empty() && plugin_points.is_empty() && resolved_plugin_points.is_empty() {
            return Ok(0);
        }

        // Phase 1: extract — gather all ConnectionPoints.
        clear_connection_points(conn)?;

        let mut all_points: Vec<ConnectionPoint> = Vec::new();

        // Ingest plugin-emitted points first so they take precedence during
        // dedup (first wins). As connectors flatten into their owning plugin
        // their legacy extract impl shrinks; this ordering keeps the
        // authoritative source ahead of any remaining regex-based legacy
        // fallback.
        if !plugin_points.is_empty() {
            info!("plugin extract_connection_points: {} points", plugin_points.len());
            all_points.extend_from_slice(plugin_points);
        }

        // Points from `LanguagePlugin::resolve_connection_points` — the
        // post-parse hook for connectors needing cross-file joins, DI
        // resolution, or class-inheritance walks that weren't available at
        // parse time.
        if !resolved_plugin_points.is_empty() {
            info!(
                "plugin resolve_connection_points: {} points",
                resolved_plugin_points.len()
            );
            all_points.extend_from_slice(resolved_plugin_points);
        }

        for connector in &active {
            let desc = connector.descriptor();
            let result = match changed_paths {
                Some(scope) => connector.incremental_extract(conn, project_root, scope),
                None => connector.extract(conn, project_root),
            };
            match result {
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
///
/// Sources (registered in this order, so plugin connectors get priority):
///   1. Language plugin connectors — from `LanguagePlugin::connectors()`
///   2. Cross-cutting connectors — shared across languages (being migrated)
pub fn build_default_registry() -> ConnectorRegistry {
    let mut reg = ConnectorRegistry::new();

    // All connectors are now provided by language plugins.
    for connector in crate::languages::collect_plugin_connectors() {
        reg.register(connector);
    }

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
