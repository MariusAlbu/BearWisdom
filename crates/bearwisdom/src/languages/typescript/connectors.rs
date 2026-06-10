// =============================================================================
// languages/typescript/connectors.rs — TypeScript connector facade
//
// Thin entry points called by the indexer:
//   - `discover_nestjs_routes`   → `connectors_nestjs`
//   - `discover_nextjs_routes`   → `connectors_nextjs`
//   - `run_react_patterns`       → `connectors_react`
//   - `extract_typescript_graphql` is re-exported from `connectors_graphql`.
//
// Also owns the cross-connector shared helpers `insert_ts_route` (used by
// both NestJS and Next.js to write a row into the `routes` table) and the
// `OptionalExt` trait used by NestJS and React lookups.
// =============================================================================

use std::path::Path;

use rusqlite::Connection;

use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;

use super::connectors_nestjs::extract_nestjs_routes;
use super::connectors_nextjs::_nextjs_routes_inner;
use super::connectors_react::{
    react_create_concepts, react_find_story_mappings, react_find_zustand_stores,
};

pub use super::connectors_graphql::extract_typescript_graphql;
pub use super::connectors_nestjs::NestRoute;
pub use super::connectors_react::{StoryMapping, ZustandStore};

// ===========================================================================
// NestJS
// ===========================================================================

/// NestJS @Controller / @Get etc. cross-file scan + `routes` table
/// population. The routes-table → FlowEmission bridge in resolve/mod.rs
/// handles downstream flow_edges emission.
///
/// Returns the count of routes written to the `routes` table.
pub fn discover_nestjs_routes(conn: &Connection, project_root: &Path, ctx: &ProjectContext) -> u32 {
    if !ctx.has_dependency(ManifestKind::Npm, "@nestjs/core")
        && !ctx.has_dependency(ManifestKind::Npm, "@nestjs/common")
    {
        return 0;
    }
    let routes = match extract_nestjs_routes(conn, project_root) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("NestJS route detection failed: {e}");
            return 0;
        }
    };
    let mut inserted: u32 = 0;
    for r in &routes {
        if insert_ts_route(
            conn,
            r.file_id,
            r.symbol_id,
            &r.http_method,
            &r.route_template,
            r.line,
        ) {
            inserted += 1;
        }
    }
    inserted
}

// ===========================================================================
// Next.js
// ===========================================================================

/// Next.js Pages Router + App Router file-based routing scan + `routes`
/// table population. The routes-table → FlowEmission bridge in
/// resolve/mod.rs handles downstream flow_edges emission.
///
/// Returns the count of routes written to the `routes` table.
pub fn discover_nextjs_routes(conn: &Connection, project_root: &Path, ctx: &ProjectContext) -> u32 {
    if !ctx.has_dependency(ManifestKind::Npm, "next") {
        return 0;
    }
    match _nextjs_routes_inner(conn, project_root) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("Next.js route detection failed: {e}");
            0
        }
    }
}

// ===========================================================================
// Shared route insert
// ===========================================================================

/// Insert a single TS-discovered route row. Returns true on insert.
pub(super) fn insert_ts_route(
    conn: &Connection,
    file_id: i64,
    symbol_id: Option<i64>,
    http_method: &str,
    route: &str,
    line: u32,
) -> bool {
    let result = conn.execute(
        "INSERT OR IGNORE INTO routes
           (file_id, symbol_id, http_method, route_template, resolved_route, line)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
        rusqlite::params![file_id, symbol_id, http_method, route, line],
    );
    matches!(result, Ok(n) if n > 0)
}

// ===========================================================================
// rusqlite optional helper
// ===========================================================================

pub(super) trait OptionalExt<T> {
    fn optional(self) -> Option<T>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> Option<T> {
        match self {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(_) => None,
        }
    }
}

// ===========================================================================
// React patterns post-index hook
// ===========================================================================

/// Detect React-ecosystem patterns (Zustand stores, Storybook stories) and
/// create concept entries.
///
/// Called from `TypeScriptPlugin::post_index()`. Non-fatal — each sub-step
/// logs warnings on failure rather than propagating errors.
pub fn run_react_patterns(conn: &rusqlite::Connection, project_root: &std::path::Path) {
    use tracing::warn;

    match react_find_zustand_stores(conn, project_root) {
        Ok(stores) => match react_find_story_mappings(conn, project_root) {
            Ok(stories) if !stores.is_empty() || !stories.is_empty() => {
                let _ = react_create_concepts(conn, &stores, &stories)
                    .map_err(|e| warn!("React concept creation: {e}"));
            }
            Err(e) => warn!("Story mapping: {e}"),
            _ => {}
        },
        Err(e) => warn!("Zustand store detection: {e}"),
    }
}
