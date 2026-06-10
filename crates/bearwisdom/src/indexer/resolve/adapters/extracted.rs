// =============================================================================
// indexer/resolve/adapters/extracted.rs — extractor → FlowEmission adapters
//
// Bridges between extractor-time data structures (`pf.routes`, `pf.db_sets`,
// `pf.plugin_flow_emissions`, persisted `routes` rows) and the resolver's
// FlowEmission pipeline. Each adapter turns one extractor surface into one
// or more emissions the cross-file pairer can consume.
// =============================================================================

use anyhow::{Context, Result};

use super::super::flow_emit;

/// Convert per-file `ExtractedRoute` entries into Consumer
/// `NamedChannel { kind: HttpCall, .. }` emissions. Used by the cross-language
/// adapter to surface HTTP routes from any language that participates in
/// `pf.routes` (C# ASP.NET attribute routes, Elixir Phoenix scope blocks,
/// Go chi/gin chains, Java Spring `@GetMapping`, …) — they pair against
/// Producer-side HTTP calls keyed on the same URL.
///
/// Returns `(handler_line, emission)` pairs. Routes whose `template` is
/// empty after trimming are skipped — without a URL there's no pairing
/// key. The `http_method` field is parsed via `HttpMethod::from_method_name`
/// so `"GET"` / `"get"` / `""` all map sensibly (`""` collapses to `Any`).
/// The URL template is canonicalised through `url_pattern::normalize` so
/// dynamic-segment variants (`{id}`, `:id`, `<id>`) compare equal across
/// languages.
pub(crate) fn extracted_routes_to_emissions(
    routes: &[crate::types::ExtractedRoute],
    symbols: &[crate::types::ExtractedSymbol],
) -> Vec<(u32, flow_emit::FlowEmission)> {
    let mut out = Vec::with_capacity(routes.len());
    for route in routes {
        if route.template.trim().is_empty() {
            continue;
        }
        let handler_line = symbols
            .get(route.handler_symbol_index)
            .map(|s| s.start_line)
            .unwrap_or(1);
        let method = flow_emit::HttpMethod::from_method_name(route.http_method.as_str());
        let name = crate::connectors::url_pattern::normalize(route.template.as_str());
        out.push((
            handler_line,
            flow_emit::FlowEmission::NamedChannel {
                kind: flow_emit::NamedChannelKind::HttpCall,
                name,
                role: flow_emit::ChannelRole::Consumer,
                method: Some(method),
                streaming: None,
            },
        ));
    }
    out
}

/// Flush extractor-time `FlowEmission`s into the resolve loop's accumulator.
/// Used for plugins whose flow detection is file-structure-based (GraphQL
/// SDL blocks, `.proto` service definitions) and runs at parse time rather
/// than during chain-walk resolver emission. The tuple shape matches
/// `extracted_routes_to_emissions` — `(file_path, line, emission)`.
pub(crate) fn plugin_flow_emissions_to_emissions(
    pf: &crate::types::ParsedFile,
    emissions: &mut Vec<(String, u32, flow_emit::FlowEmission)>,
) {
    for (line, emission) in &pf.plugin_flow_emissions {
        emissions.push((pf.path.clone(), *line, emission.clone()));
    }
}

/// Convert per-file `ExtractedDbSet` entries into `FlowEmission::DbEntity`
/// emissions. Used for languages whose model-table mapping is extracted
/// at the symbol layer (C# EF Core `DbSet<TEntity>` properties on a
/// DbContext). Each entry becomes a DbEntity row keyed by the table name
/// (or entity type name if the table name is empty), letting DbQuery
/// emissions for the same entity name pair against it via the pairer's
/// fuzzy name matching.
pub(crate) fn extracted_db_sets_to_emissions(
    db_sets: &[crate::types::ExtractedDbSet],
    symbols: &[crate::types::ExtractedSymbol],
) -> Vec<(u32, flow_emit::FlowEmission)> {
    let mut out = Vec::with_capacity(db_sets.len());
    for ds in db_sets {
        let line = symbols
            .get(ds.property_symbol_index)
            .map(|s| s.start_line)
            .unwrap_or(1);
        let table = if ds.table_name.is_empty() {
            None
        } else {
            Some(ds.table_name.clone())
        };
        out.push((
            line,
            flow_emit::FlowEmission::DbEntity {
                base_symbol_id: None,
                base_name_hint: ds.entity_type.clone(),
                table_name_hint: table,
            },
        ));
    }
    out
}

/// Append a Consumer `NamedChannel { kind: HttpCall, .. }` emission for every
/// row in the `routes` table whose file is internal and whose template is
/// non-empty. Used for languages whose route extraction is implemented as a
/// project-wide `Connector` writing to the DB directly (Go chi/gin, Java
/// Spring `@GetMapping`, Elixir Phoenix scope blocks). The per-file
/// `pf.routes`-driven adapter handles the remaining languages (C# and TS
/// NestJS). Skips routes whose host file isn't on disk (deleted between
/// extract and resolve).
pub fn append_db_route_consumer_emissions(
    conn: &rusqlite::Connection,
    emissions: &mut Vec<(String, u32, flow_emit::FlowEmission)>,
) -> Result<()> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT f.path,
                    r.http_method,
                    COALESCE(r.resolved_route, r.route_template) AS template,
                    r.line
             FROM routes r
             JOIN files f ON r.file_id = f.id
             WHERE f.origin = 'internal'",
        )
        .context("preparing routes-table read for flow adapter")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let path: String = row.get(0)?;
        let method_str: String = row.get(1)?;
        let template: String = row.get(2)?;
        let line: u32 = row.get(3).unwrap_or(1);
        if template.trim().is_empty() {
            continue;
        }
        let method = flow_emit::HttpMethod::from_method_name(method_str.as_str());
        let name = crate::connectors::url_pattern::normalize(template.as_str());
        emissions.push((
            path,
            line.max(1),
            flow_emit::FlowEmission::NamedChannel {
                kind: flow_emit::NamedChannelKind::HttpCall,
                name,
                role: flow_emit::ChannelRole::Consumer,
                method: Some(method),
                streaming: None,
            },
        ));
    }
    Ok(())
}
