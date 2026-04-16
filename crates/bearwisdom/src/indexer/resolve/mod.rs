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

pub mod chain_walker;
pub mod engine;
mod heuristic;
pub mod inheritance;
pub mod rules;
pub mod type_env;

use engine::SymbolLookup;

use crate::db::Database;
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};
use anyhow::{Context, Result};
use engine::{build_scope_chain, RefContext, ResolutionEngine, SymbolIndex};
use std::collections::HashMap;
use tracing::{debug, info, warn};

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
    resolve_and_write_inner(db, parsed, symbol_id_map, project_ctx, false)
}

/// Incremental variant: augments the SymbolIndex with all symbols from DB
/// so the engine resolver can find targets in unchanged files.
pub fn resolve_and_write_incremental(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
) -> Result<ResolutionStats> {
    resolve_and_write_inner(db, parsed, symbol_id_map, project_ctx, true)
}

fn resolve_and_write_inner(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    augment_from_db: bool,
) -> Result<ResolutionStats> {
    let engine = ResolutionEngine::new();
    let mut index = SymbolIndex::build_with_context(parsed, symbol_id_map, project_ctx);

    // For incremental: load symbols from unchanged files so the engine
    // resolver can find cross-file targets (CR #9).
    if augment_from_db {
        index.augment_from_db(db.conn());
    }

    let conn = db.conn();
    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin resolution transaction")?;

    let mut stats = ResolutionStats::default();

    // Pre-build heuristic lookup structures (needed for fallback).
    let name_to_ids = heuristic::build_name_index(symbol_id_map, parsed);
    let qname_to_id = heuristic::build_qname_index(symbol_id_map);
    let module_to_files = heuristic::build_module_to_files(parsed);
    let import_map = heuristic::build_import_map(parsed);
    let file_namespace_map = heuristic::build_file_namespace_map(parsed);

    for pf in parsed {
        // Externals are indexed for lookup only — we don't chase their internal
        // refs, otherwise a handful of third-party packages explode the edge
        // table with intra-library calls the user doesn't care about.
        if pf.path.starts_with("ext:") {
            continue;
        }

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
        let host_resolver = engine.resolver_for(&pf.language);
        let host_file_ctx = host_resolver.map(|r| r.build_file_context(pf, project_ctx));

        let empty_vec = vec![];
        let file_imports = import_map.get(&pf.path).unwrap_or(&empty_vec);
        let source_namespace = file_namespace_map.get(&pf.path).map(|s| s.as_str());

        for (ref_idx, r) in pf.refs.iter().enumerate() {
            // Determine the effective language for this ref. Refs from embedded
            // regions (e.g. TS inside a Vue/Svelte file, JS inside PHP/Elixir)
            // carry their own language tag so the resolver and
            // externals/primitives classification use the correct language's
            // ruleset rather than the host language's.
            let effective_lang: &str = pf
                .ref_origin_languages
                .get(ref_idx)
                .and_then(|o| o.as_deref())
                .unwrap_or(&pf.language);
            // Whether this ref belongs to a different language than the host.
            let is_cross_lang_embedded = effective_lang != pf.language.as_str();

            // For cross-language embedded refs, look up the resolver for the
            // embedded language. For same-language refs, reuse the host resolver
            // and file_ctx already computed for this file.
            //
            // Example: a `.vue` file (host = "vue") has `<script lang="ts">`.
            // Embedded TS refs get effective_lang = "typescript" → we use the
            // TypeScript resolver and build a fresh file_ctx from the same
            // ParsedFile (which contains all embedded symbols/imports merged in).
            let (resolver, file_ctx): (Option<&dyn engine::LanguageResolver>, _) =
                if is_cross_lang_embedded {
                    let emb_resolver = engine.resolver_for(effective_lang);
                    let emb_ctx = emb_resolver.map(|res| res.build_file_context(pf, project_ctx));
                    (emb_resolver, emb_ctx)
                } else {
                    (host_resolver, host_file_ctx.clone())
                };

            let source_id = match file_symbol_ids.get(r.source_symbol_index).and_then(|id| *id) {
                Some(id) => id,
                None => continue,
            };

            // Wildcard imports (`use foo::*`) are scope-declaration statements, not
            // missing-symbol references.  They cannot resolve to a single target and
            // should not appear in the unresolved_refs table.
            if r.kind == EdgeKind::Imports && r.target_name == "*" {
                stats.external += 1; // count as "handled" so they don't inflate unresolved rate
                continue;
            }

            // Tier 1: Try language-specific resolver (for the effective language).
            let mut resolved_by_engine = false;
            if let (Some(resolver), Some(file_ctx)) = (resolver, &file_ctx) {
                let source_sym = &pf.symbols[r.source_symbol_index];
                let ref_ctx = RefContext {
                    extracted_ref: r,
                    source_symbol: source_sym,
                    scope_chain: build_scope_chain(source_sym.scope_path.as_deref()),
                    file_package_id: pf.package_id,
                };

                if let Some(resolution) = resolver.resolve(&file_ctx, &ref_ctx, &index) {
                    let result = tx
                        .prepare_cached(
                            "INSERT OR IGNORE INTO edges
                               (source_id, target_id, kind, source_line, confidence, strategy)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        )
                        .and_then(|mut stmt| {
                            stmt.execute(rusqlite::params![
                                source_id,
                                resolution.target_symbol_id,
                                r.kind.as_str(),
                                r.line,
                                resolution.confidence,
                                resolution.strategy,
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

            // Build scope context once for remaining classification steps.
            let source_sym = &pf.symbols[r.source_symbol_index];
            let scope_chain = build_scope_chain(source_sym.scope_path.as_deref());

            // ---------------------------------------------------------------
            // Tier 1.1: Generic type parameter resolution.
            // If this is a TypeRef and the target matches a generic param
            // declared on an enclosing type (e.g., `T` in `class Repo<T>`),
            // it's a type parameter — not a missing symbol.
            // ---------------------------------------------------------------
            if r.kind == EdgeKind::TypeRef {
                let is_generic_param = scope_chain.iter().any(|scope| {
                    index
                        .generic_params(scope)
                        .map_or(false, |params| params.iter().any(|p| p == &r.target_name))
                });
                if is_generic_param {
                    tx.prepare_cached(
                        "INSERT INTO external_refs
                           (source_id, target_name, kind, source_line, namespace, package_id)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    )
                    .and_then(|mut stmt| {
                        stmt.execute(rusqlite::params![
                            source_id,
                            r.target_name,
                            r.kind.as_str(),
                            r.line,
                            "generic_param",
                            pf.package_id,
                        ])
                    })
                    .ok();
                    stats.external += 1;
                    continue;
                }
            }

            // ---------------------------------------------------------------
            // Tier 1.5: External classification — BEFORE heuristic.
            //
            // Language resolvers and chain inference can identify refs that
            // belong to external packages (stdlib, third-party crates).
            // Check this FIRST so the heuristic doesn't create false
            // low-confidence edges for things like `map`, `iter`, `get`
            // that match internal method names by coincidence.
            //
            // `resolver` and `file_ctx` here are already the effective-language
            // versions (embedded resolver for cross-lang refs, host resolver
            // otherwise), so no special-casing is needed.
            // ---------------------------------------------------------------
            let inferred_ns = if let (Some(resolver), Some(file_ctx)) = (resolver, &file_ctx) {
                let ref_ctx = RefContext {
                    extracted_ref: r,
                    source_symbol: source_sym,
                    scope_chain: scope_chain.clone(),
                    file_package_id: pf.package_id,
                };
                resolver.infer_external_namespace_with_lookup(
                    file_ctx, &ref_ctx, project_ctx, &index,
                )
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

            // Bare-name external check: test globals, language primitives,
            // and runtime builtins — classified with specific namespaces.
            // Use effective_lang so JS/TS refs embedded in Elixir/PHP/Ruby
            // host files are classified against the JS/TS primitive/builtin
            // tables, not the host language's.
            let inferred_ns = inferred_ns.or_else(|| {
                index
                    .classify_external_name(&r.target_name, effective_lang)
                    .map(|ns| ns.to_string())
            });

            // Import-based external for bare usages: if the ref's name (or its
            // leading segment, for qualified targets like `Stripe.Event`) matches
            // an entry in this file's import list whose source module has zero
            // local symbols in the project index, classify as external.
            //
            // Catches transitive-dep imports that language-specific manifest
            // checks miss — e.g. Java `import tools.jackson.databind.ObjectMapper`
            // (Jackson 3.x, pulled in via spring-boot-starter, not declared in
            // pom.xml) or Python `from sqlalchemy import Engine` (transitive of
            // sqlmodel). Language-agnostic: relies only on the project's own
            // symbol index, not on manifest lockfiles or hardcoded root lists.
            let inferred_ns = inferred_ns.or_else(|| {
                if r.module.is_some() {
                    return None;
                }
                let target = r.target_name.as_str();
                let first_segment = target.split('.').next().unwrap_or(target);
                for (imported_name, module_path_opt) in file_imports.iter() {
                    if imported_name != target && imported_name != first_segment {
                        continue;
                    }
                    let Some(module_path) = module_path_opt.as_deref() else {
                        continue;
                    };
                    if is_local_module_specifier(module_path) {
                        continue;
                    }
                    if is_module_in_project(module_path, &module_to_files, &index) {
                        continue;
                    }
                    return Some(format!("ext:{module_path}"));
                }
                None
            });

            // Module-qualified external check: if the ref has module="X" and
            // "X" is not a local file/namespace, classify as external.
            // e.g., R `dplyr::mutate` → dplyr not local → external.
            //       Erlang `lists:map` → lists not local → external.
            //       Haskell `Map.lookup` → Map not local → external.
            let inferred_ns = inferred_ns.or_else(|| {
                if let Some(module) = &r.module {
                    let mod_lower = module.to_lowercase();
                    // Full-path match: "Ecto.Changeset" or "dplyr" as-is.
                    let mut is_local = module_to_files.contains_key(module.as_str())
                        || module_to_files.contains_key(&mod_lower);

                    // Last-segment match ONLY for single-segment modules.
                    // Multi-segment modules like "Ecto.Changeset" should NOT
                    // be classified as local just because a file named
                    // "changeset.ex" exists — that's a coincidental stem match.
                    if !is_local && !module.contains('.') {
                        let last_seg = module.rsplit('.').next().unwrap_or(module);
                        let last_lower = last_seg.to_lowercase();
                        is_local = module_to_files.contains_key(last_seg)
                            || module_to_files.contains_key(&last_lower);
                    }

                    if !is_local {
                        Some(format!("ext:{module}"))
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

            if let Some(ns) = &inferred_ns {
                // Known external framework ref → external_refs table.
                if let Err(e) = tx
                    .prepare_cached(
                        "INSERT INTO external_refs
                           (source_id, target_name, kind, source_line, namespace, package_id)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    )
                    .and_then(|mut stmt| {
                        stmt.execute(rusqlite::params![
                            source_id,
                            r.target_name,
                            r.kind.as_str(),
                            r.line,
                            ns,
                            pf.package_id,
                        ])
                    })
                {
                    warn!(
                        "external_refs INSERT failed for '{}' (source_id={source_id}): {e}",
                        r.target_name
                    );
                }
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
                &module_to_files,
                symbol_id_map,
                parsed,
            );

            match resolution {
                Some((target_id, confidence, strategy)) => {
                    let result = tx
                        .prepare_cached(
                            "INSERT OR IGNORE INTO edges
                               (source_id, target_id, kind, source_line, confidence, strategy)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        )
                        .and_then(|mut stmt| {
                            stmt.execute(rusqlite::params![
                                source_id,
                                target_id,
                                r.kind.as_str(),
                                r.line,
                                confidence,
                                strategy,
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
                    // E3: propagate snippet flag from source symbol for
                    // aggregate-stats exclusion.
                    let from_snippet = pf
                        .symbol_from_snippet
                        .get(r.source_symbol_index)
                        .copied()
                        .unwrap_or(false);
                    tx.prepare_cached(
                        "INSERT INTO unresolved_refs
                           (source_id, target_name, kind, source_line, module, package_id, from_snippet)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    )
                    .and_then(|mut stmt| {
                        stmt.execute(rusqlite::params![
                            source_id,
                            r.target_name,
                            r.kind.as_str(),
                            r.line,
                            module_value,
                            pf.package_id,
                            from_snippet as i32,
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

    // Materialize incoming_edge_count on symbols for fast centrality lookups.
    // Scan edges once into a temp table (O(E)), then join-update symbols (O(S * log E_distinct)).
    // This avoids the correlated-subquery path that issues one COUNT per symbol row.
    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS _edge_counts (id INTEGER PRIMARY KEY, cnt INTEGER);
         DELETE FROM _edge_counts;
         INSERT INTO _edge_counts SELECT target_id, COUNT(*) FROM edges GROUP BY target_id;",
    )
    .context("Failed to build edge count temp table")?;
    conn.execute(
        "UPDATE symbols SET incoming_edge_count = COALESCE(
            (SELECT cnt FROM _edge_counts WHERE _edge_counts.id = symbols.id), 0)",
        [],
    )
    .context("Failed to materialize incoming_edge_count")?;
    conn.execute("DELETE FROM _edge_counts", [])
        .context("Failed to clean up edge count temp table")?;

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

/// A module specifier that points to project-local code rather than an
/// external package. Covers relative paths (`./foo`, `../foo`), rooted paths
/// (`/foo`), common monorepo aliases (`@/foo`, `~/foo`), Windows absolute
/// paths (`C:/foo`), and the most common tsconfig-style path aliases —
/// slash-containing specifiers that are not scoped npm packages.
fn is_local_module_specifier(module_path: &str) -> bool {
    if module_path.is_empty() {
        return true;
    }
    if module_path.starts_with('.')
        || module_path.starts_with('/')
        || module_path.starts_with("@/")
        || module_path.starts_with("~/")
    {
        return true;
    }
    // Windows absolute path: single letter drive + ':'
    let bytes = module_path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return true;
    }
    // Slash-containing specifier that is NOT a scoped npm package
    // (`@scope/pkg`) — likely a tsconfig `paths` alias or similar.
    // Scoped npm is still allowed as external (`@tanstack/react-query`).
    if module_path.contains('/') && !module_path.starts_with('@') {
        return true;
    }
    false
}

/// Does the project's symbol index cover this import module?
///
/// Returns true if the module appears as a local namespace (any symbol has
/// that prefix) or maps to a local file via the heuristic module-to-file map.
/// A module that walks through multiple segments (`a.b.c`) is local when any
/// of those segments is covered — this prevents a false "external" classification
/// for package-qualified imports like Python `from app.core.db import engine`.
fn is_module_in_project(
    module_path: &str,
    module_to_files: &rustc_hash::FxHashMap<String, Vec<String>>,
    index: &engine::SymbolIndex,
) -> bool {
    if module_to_files.contains_key(module_path) {
        return true;
    }
    let lower = module_path.to_lowercase();
    if lower != module_path && module_to_files.contains_key(&lower) {
        return true;
    }
    index.has_in_namespace(module_path)
}
