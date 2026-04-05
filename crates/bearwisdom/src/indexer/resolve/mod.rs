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
pub mod type_env;

use crate::db::Database;
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};
use anyhow::{Context, Result};
use engine::{build_scope_chain, RefContext, ResolutionEngine, SymbolIndex};
use std::collections::HashMap;
use tracing::{debug, info};

/// Stats returned by `resolve_and_write`.
#[derive(Debug, Clone, Default)]
pub struct ResolutionStats {
    pub resolved: u64,
    pub engine_resolved: u64,
    pub unresolved: u64,
    pub external: u64,
}

/// Resolve all references across all parsed files, writing edges,
/// unresolved refs, and external refs to the database.
///
/// Two-tier: language-specific resolvers first (1.0 confidence),
/// then heuristic fallback (0.50-0.95 confidence).
/// Unresolvable refs with a known external namespace go to `external_refs`;
/// truly unknown refs go to `unresolved_refs`.
pub fn resolve_and_write(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
) -> Result<ResolutionStats> {
    let engine = ResolutionEngine::new();
    let index = SymbolIndex::build(parsed, symbol_id_map);

    let conn = &db.conn;
    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin resolution transaction")?;

    let mut stats = ResolutionStats::default();

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
        let file_ctx = resolver.map(|r| r.build_file_context(pf, project_ctx));

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
                    let result = tx
                        .prepare_cached(
                            "INSERT OR IGNORE INTO edges
                               (source_id, target_id, kind, source_line, confidence)
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                        )
                        .and_then(|mut stmt| {
                            stmt.execute(rusqlite::params![
                                source_id,
                                resolution.target_symbol_id,
                                r.kind.as_str(),
                                r.line,
                                resolution.confidence,
                            ])
                        });
                    match result {
                        Ok(_) => {
                            stats.resolved += 1;
                            stats.engine_resolved += 1;
                            resolved_by_engine = true;
                        }
                        Err(e) => debug!("Engine edge insert failed: {e}"),
                    }
                }
            }

            if resolved_by_engine {
                continue;
            }

            // Tier 1.5: External classification — BEFORE heuristic.
            //
            // Language resolvers and chain inference can identify refs that
            // belong to external packages (stdlib, third-party crates).
            // Check this FIRST so the heuristic doesn't create false
            // low-confidence edges for things like `map`, `iter`, `get`
            // that match internal method names by coincidence.
            let source_sym = &pf.symbols[r.source_symbol_index];
            let scope_chain = build_scope_chain(source_sym.scope_path.as_deref());

            let inferred_ns = if let (Some(resolver), Some(file_ctx)) = (resolver, &file_ctx) {
                let ref_ctx = RefContext {
                    extracted_ref: r,
                    source_symbol: source_sym,
                    scope_chain: scope_chain.clone(),
                };
                resolver.infer_external_namespace(file_ctx, &ref_ctx, project_ctx)
            } else {
                None
            };

            // Chain-to-external: if the chain walks to a type not in the index,
            // classify as external (handles ORM, test framework, fluent API chains).
            let inferred_ns = inferred_ns.or_else(|| {
                r.chain.as_ref().and_then(|chain| {
                    engine::infer_external_from_chain(chain, &scope_chain, &index)
                })
            });

            if let Some(ns) = &inferred_ns {
                // Known external framework ref → external_refs table.
                tx.prepare_cached(
                    "INSERT INTO external_refs
                       (source_id, target_name, kind, source_line, namespace)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .and_then(|mut stmt| {
                    stmt.execute(rusqlite::params![
                        source_id,
                        r.target_name,
                        r.kind.as_str(),
                        r.line,
                        ns,
                    ])
                })
                .ok();
                stats.external += 1;
                continue;
            }

            // Tier 2: Heuristic fallback.
            let ref_module = r.module.as_deref();
            let chain_prefix = r.chain.as_ref().and_then(|c| {
                if c.segments.len() >= 2 {
                    Some(c.segments[c.segments.len() - 2].name.as_str())
                } else {
                    None
                }
            });
            let resolution = heuristic::resolve_ref(
                r.target_name.as_str(),
                r.kind,
                &pf.path,
                file_imports,
                source_namespace,
                chain_prefix,
                ref_module,
                &name_to_ids,
                &qname_to_id,
                symbol_id_map,
                parsed,
            );

            match resolution {
                Some((target_id, confidence)) => {
                    let result = tx
                        .prepare_cached(
                            "INSERT OR IGNORE INTO edges
                               (source_id, target_id, kind, source_line, confidence)
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                        )
                        .and_then(|mut stmt| {
                            stmt.execute(rusqlite::params![
                                source_id,
                                target_id,
                                r.kind.as_str(),
                                r.line,
                                confidence,
                            ])
                        });
                    match result {
                        Ok(_) => stats.resolved += 1,
                        Err(e) => debug!("Heuristic edge insert failed: {e}"),
                    }
                }
                None => {
                    // Truly unresolved — no external namespace identified,
                    // no heuristic match found.
                    let module_value = r.module.as_deref();
                    tx.prepare_cached(
                        "INSERT INTO unresolved_refs
                           (source_id, target_name, kind, source_line, module)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                    )
                    .and_then(|mut stmt| {
                        stmt.execute(rusqlite::params![
                            source_id,
                            r.target_name,
                            r.kind.as_str(),
                            r.line,
                            module_value,
                        ])
                    })
                    .ok();
                    stats.unresolved += 1;
                }
            }
        }
    }

    tx.commit()
        .context("Failed to commit resolution transaction")?;

    if stats.engine_resolved > 0 || stats.external > 0 {
        info!(
            "Resolution: {} by engine, {} by heuristic, {} external, {} unresolved",
            stats.engine_resolved,
            stats.resolved - stats.engine_resolved,
            stats.external,
            stats.unresolved,
        );
    }

    Ok(stats)
}
