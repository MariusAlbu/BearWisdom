// =============================================================================
// indexer/resolve/mod.rs — Reference resolution module
//
// Two-tier resolution:
//   1. Language-specific resolvers (engine.rs + rules/) — deterministic, 1.0 confidence
//   2. Heuristic fallback (heuristic.rs) — best-effort, 0.50-0.95 confidence
//
// The `resolve_and_write` function is the public entry point, called from
// `full.rs` and `incremental.rs` after symbols are written to the DB.
// =============================================================================

pub mod engine;
mod heuristic;
pub mod rules;

use crate::db::Database;
use crate::types::{EdgeKind, ParsedFile};
use anyhow::{Context, Result};
use engine::{build_scope_chain, RefContext, ResolutionEngine, SymbolIndex};
use std::collections::HashMap;
use tracing::{debug, info};

/// Resolve all references across all parsed files, writing edges and
/// unresolved refs to the database.
///
/// Two-tier: language-specific resolvers first (1.0 confidence),
/// then heuristic fallback (0.50-0.95 confidence).
///
/// Returns (resolved_count, unresolved_count).
pub fn resolve_and_write(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
) -> Result<(u64, u64)> {
    let engine = ResolutionEngine::new();
    let index = SymbolIndex::build(parsed, symbol_id_map);

    // Track how many refs the engine resolves vs heuristic.
    let mut engine_resolved = 0u64;

    // Build per-file symbol ID lookups (same as heuristic does).
    let conn = &db.conn;
    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin resolution transaction")?;

    let mut total_resolved = 0u64;
    let mut total_unresolved = 0u64;

    // Pre-build heuristic lookup structures (needed for fallback).
    let name_to_ids = heuristic::build_name_index(symbol_id_map, parsed);
    let qname_to_id = heuristic::build_qname_index(symbol_id_map);
    let import_map = heuristic::build_import_map(parsed);
    let file_namespace_map = heuristic::build_file_namespace_map(parsed);

    for pf in parsed {
        let file_symbol_ids: Vec<Option<i64>> = pf
            .symbols
            .iter()
            .map(|sym| {
                symbol_id_map
                    .get(&(pf.path.clone(), sym.qualified_name.clone()))
                    .copied()
            })
            .collect();

        // Try language-specific resolver for this file.
        let resolver = engine.resolver_for(&pf.language);
        let file_ctx = resolver.map(|r| r.build_file_context(pf));

        let empty_vec = vec![];
        let file_imports = import_map.get(&pf.path).unwrap_or(&empty_vec);
        let source_namespace = file_namespace_map.get(&pf.path).map(|s| s.as_str());

        for r in &pf.refs {
            let source_id = match file_symbol_ids.get(r.source_symbol_index).and_then(|id| *id) {
                Some(id) => id,
                None => continue,
            };

            // Tier 1: Try language-specific resolver.
            let mut resolved_by_engine = false;
            if let (Some(resolver), Some(file_ctx)) = (resolver, &file_ctx) {
                let source_sym = &pf.symbols[r.source_symbol_index];
                let ref_ctx = RefContext {
                    extracted_ref: r,
                    source_symbol: source_sym,
                    scope_chain: build_scope_chain(source_sym.scope_path.as_deref()),
                };

                if let Some(resolution) = resolver.resolve(file_ctx, &ref_ctx, &index) {
                    let result = tx.execute(
                        "INSERT OR IGNORE INTO edges
                           (source_id, target_id, kind, source_line, confidence)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![
                            source_id,
                            resolution.target_symbol_id,
                            r.kind.as_str(),
                            r.line,
                            resolution.confidence,
                        ],
                    );
                    match result {
                        Ok(_) => {
                            total_resolved += 1;
                            engine_resolved += 1;
                            resolved_by_engine = true;
                        }
                        Err(e) => debug!("Engine edge insert failed: {e}"),
                    }
                }
            }

            if resolved_by_engine {
                continue;
            }

            // Tier 2: Heuristic fallback.
            let resolution = heuristic::resolve_ref(
                r.target_name.as_str(),
                r.kind,
                &pf.path,
                file_imports,
                source_namespace,
                &name_to_ids,
                &qname_to_id,
                symbol_id_map,
                parsed,
            );

            match resolution {
                Some((target_id, confidence)) => {
                    let result = tx.execute(
                        "INSERT OR IGNORE INTO edges
                           (source_id, target_id, kind, source_line, confidence)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![source_id, target_id, r.kind.as_str(), r.line, confidence],
                    );
                    match result {
                        Ok(_) => total_resolved += 1,
                        Err(e) => debug!("Heuristic edge insert failed: {e}"),
                    }
                }
                None => {
                    // Try to infer which external namespace this ref comes from.
                    let inferred_module = if let (Some(resolver), Some(file_ctx)) = (resolver, &file_ctx) {
                        let source_sym = &pf.symbols[r.source_symbol_index];
                        let ref_ctx = RefContext {
                            extracted_ref: r,
                            source_symbol: source_sym,
                            scope_chain: build_scope_chain(source_sym.scope_path.as_deref()),
                        };
                        resolver.infer_external_namespace(file_ctx, &ref_ctx)
                    } else {
                        None
                    };

                    // Use inferred namespace if available, otherwise fall back to ref's module
                    let module_value = inferred_module.as_deref().or(r.module.as_deref());

                    tx.execute(
                        "INSERT INTO unresolved_refs
                           (source_id, target_name, kind, source_line, module)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![source_id, r.target_name, r.kind.as_str(), r.line, module_value],
                    )
                    .ok();
                    total_unresolved += 1;
                }
            }
        }
    }

    tx.commit()
        .context("Failed to commit resolution transaction")?;

    if engine_resolved > 0 {
        info!(
            "Resolution: {} by engine, {} by heuristic, {} unresolved",
            engine_resolved,
            total_resolved - engine_resolved,
            total_unresolved
        );
    }

    Ok((total_resolved, total_unresolved))
}
