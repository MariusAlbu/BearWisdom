// =============================================================================
// indexer/resolve/loop_body.rs — per-file rayon resolve loop
//
// The implementation of one resolution pass over a parsed-file slice. Walks
// every file's refs in parallel, runs the tier-1 language resolver, then the
// tier-1.1 generic-param probe, then the tier-1.5 external-classification
// cascade, then the tier-2 heuristic fallback, then a final classification
// for Imports-kind refs. Each rayon worker fills its own `FileWriteBuf` /
// `FileStats`; the reduce step merges them and the main thread does the
// bulk SQL flush.
//
// Public entry points stay in `mod.rs`; this file owns
// `resolve_iteration_inner` / `resolve_iteration_inner_with_index` /
// `resolve_iteration_body` plus the small helpers they need
// (`is_module_in_project` / `read_external_file_paths` /
// `read_file_imports_from_db`).
// =============================================================================

use std::collections::HashMap;

use anyhow::{Context, Result};
use rayon::prelude::*;
use tracing::info;

use crate::connectors::url_pattern;
use crate::db::Database;
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

use super::adapters::{
    extracted_db_sets_to_emissions, extracted_routes_to_emissions, mailer_template_name_for_path,
    nextjs_route_consumer_emissions, plugin_flow_emissions_to_emissions,
};
use super::legacy::{
    self, build_scope_chain, ImportEntry, RefContext, SymbolIndex, SymbolLookup,
};
use super::flow_emit;
use super::flow_pair::flush_flow_emissions;
use super::indexes;
use super::write_buf::{flush_resolve_buf, DeferredSpeculative, FileStats, FileWriteBuf};
use super::ResolutionStats;

/// Join harvested return-type candidates into per-function inferred returns.
///
/// Each candidate is `(function_qname, function_db_id, yield_type)`. A qname's
/// return is inferred only when every candidate for it agrees on a single type.
/// Agreement is the soundness gate for the qname-keyed type store, and it holds
/// even when the qname is shared across files/modules: an agreed type is correct
/// for every owner, so the same library function copied across monorepo packages
/// (identical `qname`, distinct `db_id`, same return) infers correctly — while
/// any disagreement (different return types, or an internally-ambiguous
/// multi-type return) leaves the qname uninferred, since one shared slot cannot
/// hold two types. `already_known(qname)` drops a qname that already carries a
/// declared or previously-inferred return (inference only fills genuine gaps).
///
/// Per-module inferred returns (distinct types for the same qname in different
/// modules) require the `(ModuleId, qname)`-keyed store — see MODULE-IDENTITY.md;
/// until that lands, cross-module disagreement is conservatively skipped.
pub(super) fn join_inferred_returns(
    candidates: &[(String, i64, String)],
    already_known: impl Fn(&str) -> bool,
) -> HashMap<String, String> {
    // qname → Some(agreed type) | None (conflict sentinel). The db_id is not a
    // gate: agreement across distinct owners is sound for the shared qname slot.
    let mut by_fn: HashMap<&str, Option<&str>> = HashMap::new();
    for (qname, _db_id, ty) in candidates {
        match by_fn.get(qname.as_str()) {
            None => {
                by_fn.insert(qname, Some(ty));
            }
            Some(Some(prev)) if *prev != ty.as_str() => {
                by_fn.insert(qname, None);
            }
            _ => {}
        }
    }
    let mut out = HashMap::new();
    for (qname, agreed) in by_fn {
        if let Some(ty) = agreed {
            if !already_known(qname) {
                out.insert(qname.to_string(), ty.to_string());
            }
        }
    }
    out
}

/// The resolve-loop side-tables, built once on iteration 0 and reused across the
/// fixpoint. The expand loop appends only `ext:` files, so the four
/// name/import/namespace tables (which skip `ext:`) stay stable and are reused,
/// while `name_to_ids` (keeps a few `ext:` global carriers) and
/// `engine_sym_id_map` (ext-inclusive) are extended with the appended files.
pub(crate) struct ResolveSideTables {
    name_to_ids: rustc_hash::FxHashMap<String, Vec<(String, String, String, i64)>>,
    qname_to_id: rustc_hash::FxHashMap<String, i64>,
    module_to_files: rustc_hash::FxHashMap<String, Vec<String>>,
    import_map: rustc_hash::FxHashMap<String, Vec<(String, Option<String>)>>,
    file_namespace_map: rustc_hash::FxHashMap<String, String>,
    engine_sym_id_map: crate::type_checker::core::SymbolIdMap,
}

pub(super) fn resolve_iteration_inner(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    augment_from_db: bool,
) -> Result<ResolutionStats> {
    resolve_iteration_inner_with_arena(
        db,
        parsed,
        symbol_id_map,
        project_ctx,
        augment_from_db,
        std::sync::Arc::new(crate::type_checker::core::types::TypeArena::new()),
    )
}

pub(super) fn resolve_iteration_inner_with_arena(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    augment_from_db: bool,
    type_arena: std::sync::Arc<crate::type_checker::core::types::TypeArena>,
) -> Result<ResolutionStats> {
    // Ext-origin files without an `ext:` path prefix — specifically,
    // script-tag-parsed vendored JS like `wwwroot/lib/jquery.min.js` — live
    // under regular project-relative paths in the DB but carry
    // `origin='external'`. The chain walker's "is this root internal?"
    // filter needs to see them as external so `$`-rooted jQuery chains in
    // user JS classify correctly instead of matching against those vendor
    // symbols as if they were project code.
    let external_paths = read_external_file_paths(db.conn());
    let mut index = SymbolIndex::build_with_context_and_arena(
        parsed,
        symbol_id_map,
        project_ctx,
        type_arena,
        std::sync::Arc::new(crate::ecosystem::symbol_index::SymbolLocationIndex::new()),
    );
    if !external_paths.is_empty() {
        index.set_external_paths(external_paths);
    }

    // For incremental: load symbols from unchanged files so the engine
    // resolver can find cross-file targets (CR #9). The augment SELECT
    // also collects (path, qname) → id pairs so the heuristic gets
    // project-wide coverage from the same scan — eliminates the separate
    // `load_symbol_id_map` full DB scan that incremental.rs used to do.
    let augmented_id_map: Option<HashMap<(String, String), i64>> = if augment_from_db {
        Some(index.augment_from_db_collecting_ids(db.conn()))
    } else {
        None
    };

    // One-shot / incremental path: no cross-pass caching, build once.
    let mut local_engine: Option<crate::type_checker::Engine<'static>> = None;
    let mut local_side_tables: Option<ResolveSideTables> = None;
    resolve_iteration_body(
        db,
        parsed,
        symbol_id_map,
        project_ctx,
        &mut index,
        augmented_id_map,
        &mut local_engine,
        &mut local_side_tables,
        &[],
        // One-shot / incremental: persist speculative rows immediately.
        None,
        // One-shot / incremental: resolve every internal file, no worklist.
        None,
    )
}

pub(super) fn resolve_iteration_inner_with_index(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    index: &mut SymbolIndex,
    cached_engine: &mut Option<crate::type_checker::Engine<'static>>,
    cached_side_tables: &mut Option<ResolveSideTables>,
    new_files: &[ParsedFile],
    defer_speculative: Option<&mut DeferredSpeculative>,
    retry_files: Option<&std::collections::HashSet<String>>,
) -> Result<ResolutionStats> {
    resolve_iteration_body(
        db,
        parsed,
        symbol_id_map,
        project_ctx,
        index,
        None,
        cached_engine,
        cached_side_tables,
        new_files,
        defer_speculative,
        retry_files,
    )
}

fn resolve_iteration_body(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    index: &mut SymbolIndex,
    augmented_id_map: Option<HashMap<(String, String), i64>>,
    cached_engine: &mut Option<crate::type_checker::Engine<'static>>,
    cached_side_tables: &mut Option<ResolveSideTables>,
    new_files: &[ParsedFile],
    // When `Some`, edges are flushed (they accumulate via INSERT OR IGNORE) but
    // the speculative unresolved / external rows are stashed here for the caller
    // to flush once after the fixpoint settles. `None` flushes everything
    // immediately (the one-shot / incremental path).
    defer_speculative: Option<&mut DeferredSpeculative>,
    // Worklist filter for the full-index fixpoint. `None` resolves every
    // internal file (iteration 0, incremental, one-shot). `Some(set)` resolves
    // only the frontier — files that recorded a chain miss last pass — because
    // a file with no chain misses is fully resolved and its result cannot change
    // when only `ext:` files are added to the index.
    retry_files: Option<&std::collections::HashSet<String>>,
) -> Result<ResolutionStats> {
    // The closure passed to par_iter requires `&SymbolIndex` (for the
    // SymbolLookup trait), not `&mut SymbolIndex`. Reborrow as
    // immutable for the duration of the loop.
    let index: &SymbolIndex = &*index;

    let conn = db.conn();
    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin resolution transaction")?;

    let mut stats = ResolutionStats::default();

    // Build heuristic lookup structures from the merged symbol map.
    // For full reindex, `symbol_id_map` already covers everything.
    // For incremental, we merge in the `augmented_id_map` so heuristic
    // sees both changed-file IDs (from caller) and unchanged-file IDs
    // (from the augment SELECT) without paying for two full scans.
    let merged_id_map: HashMap<(String, String), i64>;
    let merged_id_map_ref: &HashMap<(String, String), i64> = match augmented_id_map {
        Some(mut m) => {
            m.extend(symbol_id_map.iter().map(|(k, v)| (k.clone(), *v)));
            merged_id_map = m;
            &merged_id_map
        }
        None => symbol_id_map,
    };
    // Build the resolve-loop side-tables once on iteration 0 and reuse / extend
    // them across the fixpoint. The four name/import/namespace tables filter out
    // `ext:` files and the expand loop only appends `ext:` files, so they are
    // stable across expand passes and reused as-is; `name_to_ids` (keeps a few
    // `ext:` global declaration carriers) and the engine's `(path,idx)->id` map
    // (ext-inclusive, consumed by the engine build/augment below) are extended
    // with the appended files. Byte-identical because `parsed` is append-only.
    let build_engine_sym_ids = |files: &[ParsedFile]| {
        let mut map = crate::type_checker::core::SymbolIdMap::default();
        for pf in files {
            for (idx, sym) in pf.symbols.iter().enumerate() {
                if let Some(&id) =
                    merged_id_map_ref.get(&(pf.path.clone(), sym.qualified_name.clone()))
                {
                    map.insert((pf.path.clone(), idx), id);
                }
            }
        }
        map
    };
    if cached_side_tables.is_none() {
        *cached_side_tables = Some(ResolveSideTables {
            name_to_ids: indexes::build_name_index(merged_id_map_ref, parsed),
            qname_to_id: indexes::build_qname_index(merged_id_map_ref),
            module_to_files: indexes::build_module_to_files(parsed),
            import_map: indexes::build_import_map(parsed),
            file_namespace_map: indexes::build_file_namespace_map(parsed),
            engine_sym_id_map: build_engine_sym_ids(parsed),
        });
    } else if !new_files.is_empty() {
        let st = cached_side_tables
            .as_mut()
            .expect("cached_side_tables is Some");
        for (k, v) in indexes::build_name_index(merged_id_map_ref, new_files) {
            st.name_to_ids.entry(k).or_default().extend(v);
        }
        st.engine_sym_id_map.extend(build_engine_sym_ids(new_files));
    }
    let st = cached_side_tables
        .as_ref()
        .expect("cached_side_tables is set above");
    let name_to_ids = &st.name_to_ids;
    let qname_to_id = &st.qname_to_id;
    let module_to_files = &st.module_to_files;
    let import_map = &st.import_map;
    let file_namespace_map = &st.file_namespace_map;
    let engine_sym_id_map = &st.engine_sym_id_map;
    // Build the Engine once and augment it with each expand iteration's appended
    // files. The return-inference loop appends no files and reuses the Engine
    // untouched (inferred returns flow through the lookup's `return_type_name`,
    // which the SymbolIndex patches). The one-shot / incremental callers pass a
    // fresh `&mut None`, so they build once per call.
    if cached_engine.is_none() {
        *cached_engine = Some(crate::type_checker::Engine::build_from_registry(
            parsed,
            engine_sym_id_map,
            index,
            index.type_arena_arc(),
        ));
    } else if !new_files.is_empty() {
        cached_engine
            .as_mut()
            .expect("cached_engine is Some")
            .augment(parsed, new_files, engine_sym_id_map, index);
    }
    let type_engine = cached_engine.as_ref().expect("cached_engine is set above");

    // Fast companion lookup: HashMap<path, &ParsedFile> replaces the
    // per-iteration O(N) `parsed.iter().find` scan that resolve used
    // before. Bonus: when a companion file ISN'T in `parsed` (incremental
    // run — only the template changed, the component is in the DB), we
    // fall back to pulling that companion's imports directly from the
    // `imports` DB table. Without that fallback, an Angular template
    // edited on its own lost every inherited `.component.ts` import and
    // dropped from ~90% to ~52% resolution rate on touched files.
    let parsed_by_path: std::collections::HashMap<&str, &ParsedFile> =
        parsed.iter().map(|p| (p.path.as_str(), p)).collect();

    // Pre-fetch companion-file imports from DB for files whose companion
    // isn't in this run's parse slice (incremental: companion unchanged,
    // not re-parsed). Keeps the inner per-file loop free of `&Connection`
    // so it can run on rayon workers.
    let companion_db_imports: HashMap<String, Vec<ImportEntry>> = {
        let plugin_registry = crate::languages::default_registry();
        let mut needed: std::collections::HashSet<String> = std::collections::HashSet::new();
        for pf in parsed {
            if pf.path.starts_with("ext:") {
                continue;
            }
            if let Some(plugin) = plugin_registry.get_dedicated(&pf.language) {
                if let Some(companion_path) = plugin.companion_file_for_imports(&pf.path) {
                    if !parsed_by_path.contains_key(companion_path.as_str()) {
                        needed.insert(companion_path);
                    }
                }
            }
        }
        let mut map: HashMap<String, Vec<ImportEntry>> = HashMap::with_capacity(needed.len());
        for path in needed {
            let imports = read_file_imports_from_db(conn, &path);
            if !imports.is_empty() {
                map.insert(path, imports);
            }
        }
        map
    };

    // Per-file resolve runs in parallel via rayon — each worker owns its
    // own `FileWriteBuf` + `FileStats`; results are reduced into the
    // global combined values. The shared inputs (`engine`, `index`,
    // `project_ctx`, all the lookup maps, `parsed_by_path`) are
    // immutable after build so they can be passed by `&` across threads.
    // `SymbolIndex.local_type_cache` is thread-local; chain misses go
    // through a Mutex (bounded contention). The closure captures only
    // `Send + Sync` state.
    //
    // External (`ext:`) files are filtered out — they're indexed for
    // lookup only, never as resolution sources.
    let (mut combined_buf, local_stats_total) = crate::indexer::parse_file::with_resolve_pool(|| parsed
        .par_iter()
        .filter(|pf| {
            // `ext:` files are lookup targets only, never resolution sources.
            // With a worklist, also skip any internal file that recorded no
            // chain miss last pass — its result is stable.
            !pf.path.starts_with("ext:")
                && retry_files.map_or(true, |s| s.contains(pf.path.as_str()))
        })
        .map(|pf| -> (FileWriteBuf, FileStats) {
            let mut buf = FileWriteBuf::default();
            let mut local_stats = FileStats::default();

            // File-path Consumer pass for mailer templates. A file under a
            // recognised template root (`mails/`, `emails/`, `templates/email/`,
            // `views/mails/`) emits a single Consumer NamedChannel Mailer
            // keyed on the file's basename so a Producer that names the same
            // template pairs against it. Runs once per file at line 1.
            if let Some(template_name) = mailer_template_name_for_path(&pf.path) {
                buf.flow_emissions.push((
                    pf.path.clone(),
                    1u32,
                    flow_emit::FlowEmission::NamedChannel {
                        kind: flow_emit::NamedChannelKind::Mailer,
                        name: template_name,
                        role: flow_emit::ChannelRole::Consumer,
                        method: None,
                        streaming: None,
                    },
                ));
            }

            // File-path Consumer pass for Next.js routes. App Router files
            // matching `**/app/**/route.{ts,tsx,js,jsx}` emit one Consumer
            // per exported HTTP-verb handler (`GET`/`POST`/…). Pages Router
            // files matching `**/pages/api/**/*.{ts,…}` emit a single Any-
            // method Consumer keyed on the file's URL path.
            for emission in nextjs_route_consumer_emissions(&pf.path, &pf.symbols) {
                buf.flow_emissions.push((pf.path.clone(), 1u32, emission));
            }

            // Cross-language adapter: every `ExtractedRoute` populated by a
            // language extractor (ASP.NET attribute routes, Phoenix routes,
            // chi/gin/echo routes, Spring `@GetMapping`, etc.) becomes a
            // Consumer NamedChannel HttpCall so it can pair against
            // Producer-side HTTP calls from any other language.
            for (line, emission) in extracted_routes_to_emissions(&pf.routes, &pf.symbols) {
                buf.flow_emissions.push((pf.path.clone(), line, emission));
            }

            // Extractor-time `FlowEmission`s — populated at parse time by
            // file-structure scanners that can't migrate to chain-walk
            // resolver-time emission (GraphQL SDL parsing, `.proto` service
            // blocks).
            plugin_flow_emissions_to_emissions(pf, &mut buf.flow_emissions);

            // ExtractedDbSet → DbEntity adapter. Languages with model-table
            // extraction (currently C# EF Core via `DbSet<T>` properties on
            // DbContext classes) emit one ExtractedDbSet per discovered
            // model. Each becomes a `FlowEmission::DbEntity` so DbQuery
            // emissions for the same entity name pair against it via the
            // pairer's entity matcher.
            for (line, emission) in extracted_db_sets_to_emissions(&pf.db_sets, &pf.symbols) {
                buf.flow_emissions.push((pf.path.clone(), line, emission));
            }

            // Look up source IDs against the MERGED map so incremental
            // resolves can find symbol IDs for files that weren't in this
            // run's `parsed` slice (e.g. blast-radius files re-parsed for
            // resolve but whose IDs come from the augment SELECT, not from
            // the caller's changed-only map).
            let file_symbol_ids: Vec<Option<i64>> = pf
                .symbols
                .iter()
                .map(|sym| {
                    merged_id_map_ref
                        .get(&(pf.path.clone(), sym.qualified_name.clone()))
                        .copied()
                })
                .collect();

            // Build the per-file resolution context purely via the engine hook.
            let host_plugin = crate::languages::default_registry().get_dedicated(&pf.language);
            let host_file_ctx = type_engine
                .build_file_context(&pf.language, pf, project_ctx)
                .map(|mut ctx| {
                    // Companion imports: Angular template inherits the paired
                    // `.component.ts` imports. Lives on
                    // `LanguagePlugin::companion_file_for_imports`.
                    if let Some(companion_path) =
                        host_plugin.and_then(|p| p.companion_file_for_imports(&pf.path))
                    {
                        if let Some(comp_pf) = parsed_by_path.get(companion_path.as_str()) {
                            if let Some(comp_ctx) = type_engine.build_file_context(
                                &comp_pf.language,
                                comp_pf,
                                project_ctx,
                            ) {
                                ctx.imports.extend(comp_ctx.imports);
                            }
                        } else if let Some(db_imports) = companion_db_imports.get(&companion_path) {
                            ctx.imports.extend(db_imports.iter().cloned());
                        }
                    }
                    ctx
                });

            let empty_vec = vec![];
            let file_imports = import_map.get(&pf.path).unwrap_or(&empty_vec);
            let source_namespace = file_namespace_map.get(&pf.path).map(|s| s.as_str());

            // R5: install a fresh per-file flow-typing cache. Narrowings are
            // sorted innermost-first (smallest range first) so the cache's
            // cursor-based lookup picks the most specific scope on ties.
            let mut narrowings = pf.flow.narrowings.clone();
            narrowings.sort_by_key(|n| n.byte_end.saturating_sub(n.byte_start));
            let mut discriminants = pf.flow.discriminant_narrowings.clone();
            discriminants.sort_by_key(|d| d.byte_end.saturating_sub(d.byte_start));
            index.install_local_cache(narrowings, discriminants, pf.flow.cfg.clone());

            // R5: seed local types from explicit annotations (`let x: T`). Unlike
            // forward inference these need no RHS to resolve — the annotation is
            // the type — so a chain whose receiver is an annotated local resolves
            // even when its initializer (`expr()?`, `.unwrap()`) doesn't. The ref
            // loop's resolved-RHS writes overwrite in source order.
            for (&lhs_idx, decl_type) in &pf.flow.flow_binding_decl_type {
                if let Some(lhs_sym) = pf.symbols.get(lhs_idx) {
                    index.record_local_type(lhs_sym.name.clone(), decl_type.clone());
                }
            }

            // Clear this worker's chain-miss accumulator so the misses recorded
            // in the ref loop below attribute to this file. Must come after the
            // local-type seed pass and before the ref iteration below.
            index.reset_file_misses();

            // R5: iterate refs in source order so forward inference
            // (`let x = foo(); x.bar()`) propagates correctly. Reassignment is
            // handled naturally by last-write-wins in the cache. We keep the
            // original ref_idx alongside so flow_binding_lhs / origin-language
            // lookups stay correct after the sort.
            //
            // Only sort when the file actually uses flow-typing — the sort
            // perturbs extractor emission order, which INSERT OR IGNORE on
            // edges is sensitive to for duplicate-target refs. When no flow
            // metadata is present, preserve the original order.
            let uses_flow = !pf.flow.flow_binding_lhs.is_empty() || !pf.flow.narrowings.is_empty();
            let refs_ordered: Vec<(usize, &crate::types::ExtractedRef)> = if uses_flow {
                let mut v: Vec<_> = pf.refs.iter().enumerate().collect();
                v.sort_by_key(|(_, r)| r.line);
                v
            } else {
                pf.refs.iter().enumerate().collect()
            };

            for (ref_idx, r) in refs_ordered {
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
                // Same-language refs (the common case) borrow the per-file context
                // built once above; only cross-language embedded refs need a fresh,
                // owned context for the embedded language's resolver.
                let embedded_file_ctx: Option<legacy::FileContext> = if is_cross_lang_embedded {
                    type_engine.build_file_context(effective_lang, pf, project_ctx)
                } else {
                    None
                };
                let file_ctx: Option<&legacy::FileContext> = if is_cross_lang_embedded {
                    embedded_file_ctx.as_ref()
                } else {
                    host_file_ctx.as_ref()
                };

                let source_id = match file_symbol_ids
                    .get(r.source_symbol_index)
                    .and_then(|id| *id)
                {
                    Some(id) => id,
                    None => continue,
                };

                // Wildcard imports (`use foo::*`) are scope-declaration statements, not
                // missing-symbol references.  They cannot resolve to a single target and
                // should not appear in the unresolved_refs table.
                if r.kind == EdgeKind::Imports && r.target_name == "*" {
                    local_stats.external += 1; // count as "handled" so they don't inflate unresolved rate
                    continue;
                }

                // R5: move the flow-cache cursor to this ref's byte offset so
                // narrowing lookups in chain walkers see the right scope.
                // Sprint 1 leaves this as 0 for languages that haven't wired
                // their FlowConfig yet — narrowings are empty in that case so
                // the cursor value doesn't matter.
                let ref_byte = pf.flow.ref_byte_offsets.get(ref_idx).copied().unwrap_or(0);
                index.set_cursor(ref_byte);

                // Built once per ref and shared by tier-1 resolution, the
                // compiler-resolve reroute check, and tier-1.5 external
                // classification — all of which need the same scope chain and
                // source symbol.
                let source_sym = &pf.symbols[r.source_symbol_index];
                let ref_ctx = RefContext {
                    extracted_ref: r,
                    source_symbol: source_sym,
                    scope_chain: build_scope_chain(source_sym.scope_path.as_deref()),
                    file_package_id: pf.package_id,
                };

                // Tier 1: Try language-specific resolver (for the effective language).
                let mut resolved_by_engine = false;
                if let Some(file_ctx) = file_ctx {
                    // Flow-emission detection runs regardless of whether resolution
                    // succeeds — HTTP client calls, IPC, WebSocket emits, etc. are
                    // identifiable from import context alone, even when the chain
                    // walker can't resolve the external symbol to a DB id.
                    for emission in type_engine.detect_flow_emissions(file_ctx, &ref_ctx, index) {
                        buf.flow_emissions.push((pf.path.clone(), r.line, emission));
                    }

                    // Resolution dispatches through `type_engine.resolve`. The
                    // engine routes chain-bearing refs through the unified
                    // chain walker (when a profile is registered), chain-less
                    // refs through the bare-name resolver, and falls back to
                    // the language hook's `resolve_ref` (the absorbed legacy
                    // resolver body) for everything the engine declines.
                    let resolution = type_engine
                        .resolve(&ref_ctx, file_ctx, index)
                        // Embedded-origin miss → host-hook fallback. A cross-lang
                        // embedded ref dispatches against the ORIGIN language's
                        // file_ctx, so a host plugin's scope-directed binding (a
                        // HEEx template helper → its co-located Phoenix `*View`
                        // function) is unreachable through the engine path. On a
                        // miss, retry once through the HOST language's hook with
                        // the host file context. Strict widening: fires only for
                        // cross-lang embedded refs that the engine declined; the
                        // host hook either has no `resolve_ref` or is
                        // scope-directed and declines for everything else.
                        .or_else(|| {
                            if !is_cross_lang_embedded {
                                return None;
                            }
                            let host_ctx = host_file_ctx.as_ref()?;
                            type_engine.resolve_ref_via_hook(
                                &pf.language,
                                host_ctx,
                                &ref_ctx,
                                index,
                            )
                        })
                        // Inline external materialization at the resolution-
                        // failure point — the precise signal the lookup layer
                        // can't see. The engine + hook both declined, so the
                        // target may be an external symbol whose defining file
                        // isn't pulled yet. Materialize the file(s) the location
                        // index says define this name (expand's chain-miss
                        // closure, applied inline) and retry the resolution once.
                        // No-op when the name is unknown to the location index,
                        // so a genuinely-unresolvable ref still falls through.
                        .or_else(|| {
                            index.materialize_by_name(&r.target_name);
                            type_engine.resolve(&ref_ctx, file_ctx, index)
                        })
                        .map(|r| (r, true));

                    if let Some((resolution, came_from_engine)) = resolution {
                        // R5 forward-inference cache write. Engine yields are
                        // recorded only when the engine path provided them;
                        // legacy resolutions still drive the cache as before.
                        // We don't gate this on came_from_engine because both
                        // paths populate `resolved_yield_type` correctly when
                        // they can — letting both feed the cache keeps yield
                        // inference uniform across resolution sources.
                        if let Some(lhs_idx) = pf.flow.flow_binding_lhs.get(&ref_idx).copied() {
                            let yield_str = resolution
                                .resolved_yield_type
                                .and_then(|id| {
                                    index.type_arena().map(|arena| arena.format_type(id))
                                })
                                .or_else(|| {
                                    let target_id = resolution.target_symbol_id;
                                    index
                                        .by_name(&r.target_name)
                                        .iter()
                                        .find(|s| s.id == target_id)
                                        .and_then(|s| {
                                            index
                                                .return_type_str(&s.qualified_name)
                                                .or_else(|| index.field_type_str(&s.qualified_name))
                                        })
                                })
                                // Construction initializer (`def x = new C(...)`):
                                // a resolved Instantiates ref yields no return /
                                // field type (the target is a class), so derive
                                // the local's type from the constructed name. The
                                // engine's expression-type inference maps an
                                // Instantiates ref to its class TypeId.
                                .or_else(|| {
                                    type_engine
                                        .infer_yield(r, Some(&resolution), effective_lang)
                                        .and_then(|id| {
                                            index.type_arena().map(|arena| arena.format_type(id))
                                        })
                                });
                            if let Some(yield_str) = yield_str {
                                // A `?`-unwrapped binding (`let x = expr()?`) yields
                                // the wrapper's payload — peel one layer so `x` is
                                // typed as `T`, not `Result<T>`.
                                let recorded = if pf.flow.flow_binding_unwrap.contains(&lhs_idx) {
                                    legacy::first_generic_arg(&yield_str).unwrap_or(yield_str)
                                } else {
                                    yield_str
                                };
                                if let Some(lhs_sym) = pf.symbols.get(lhs_idx) {
                                    index.record_local_type(lhs_sym.name.clone(), recorded);
                                }
                            }
                        }

                        // INFER-3: harvest a return-type candidate. When this ref
                        // is a `return <expr>` of a function with no declared or
                        // already-known return type, record its resolved yield as a
                        // candidate; the orchestrator joins candidates per function
                        // (conflict → skip) and gap-fills the index, re-resolving so
                        // callers read the inferred return.
                        if let Some(fn_idx) = pf.flow.flow_return_lhs.get(&ref_idx).copied() {
                            // The function's own DB id keys the candidate so the
                            // join can detect a qname claimed by more than one
                            // function (same simple name in different files) and
                            // skip it — inference would be unsound there.
                            let fn_db_id = file_symbol_ids.get(fn_idx).and_then(|id| *id);
                            if let (Some(fn_sym), Some(fn_db_id)) =
                                (pf.symbols.get(fn_idx), fn_db_id)
                            {
                                if fn_sym.return_type.is_none()
                                    && index.return_type_name(&fn_sym.qualified_name).is_none()
                                {
                                    let yield_str = resolution
                                        .resolved_yield_type
                                        .and_then(|id| {
                                            index.type_arena().map(|arena| arena.format_type(id))
                                        })
                                        .or_else(|| {
                                            let target_id = resolution.target_symbol_id;
                                            index
                                                .by_name(&r.target_name)
                                                .iter()
                                                .find(|s| s.id == target_id)
                                                .and_then(|s| {
                                                    index
                                                        .return_type_str(&s.qualified_name)
                                                        .or_else(|| {
                                                            index.field_type_str(&s.qualified_name)
                                                        })
                                                })
                                        });
                                    if let Some(ys) = yield_str {
                                        // Skip the Unknown sentinel (format_type
                                        // emits lowercase "unknown") and a bare
                                        // generic-parameter name (e.g. `T`) — neither
                                        // is a real, bindable return type.
                                        let is_generic_param = index
                                            .generic_params(&fn_sym.qualified_name)
                                            .map_or(false, |g| g.iter().any(|p| p == &ys));
                                        if !ys.is_empty()
                                            && !ys.eq_ignore_ascii_case("unknown")
                                            && !is_generic_param
                                        {
                                            buf.inferred_returns.push((
                                                fn_sym.qualified_name.clone(),
                                                fn_db_id,
                                                ys,
                                            ));
                                        }
                                    }
                                }
                            }
                        }

                        buf.edges.push((
                            source_id,
                            resolution.target_symbol_id,
                            r.kind.as_str(),
                            r.line,
                            resolution.confidence,
                            resolution.strategy,
                        ));
                        if let Some(emission) = resolution.flow_emit {
                            buf.flow_emissions.push((pf.path.clone(), r.line, emission));
                        }
                        local_stats.resolved += 1;
                        if came_from_engine {
                            local_stats.engine_resolved += 1;
                        }
                        resolved_by_engine = true;
                    }
                }

                if resolved_by_engine {
                    continue;
                }

                // ---------------------------------------------------------------
                // Tier 1.1: Generic type parameter resolution.
                // If this is a TypeRef and the target matches a generic param
                // declared on an enclosing type (e.g., `T` in `class Repo<T>`),
                // it's a type parameter — not a missing symbol.
                // ---------------------------------------------------------------
                if r.kind == EdgeKind::TypeRef {
                    // Walk the source symbol's own qualified name first, then up
                    // through its parents. The function/struct itself owns its
                    // type parameters; refs in its signature have its qname (not
                    // its parent) as the relevant scope for generic-param lookup.
                    let is_generic_param = std::iter::once(source_sym.qualified_name.as_str())
                        .chain(ref_ctx.scope_chain.iter().map(String::as_str))
                        .any(|scope| {
                            index
                                .generic_params(scope)
                                .map_or(false, |params| params.iter().any(|p| p == &r.target_name))
                        });
                    if is_generic_param {
                        buf.externals.push((
                            source_id,
                            r.target_name.clone(),
                            r.kind.as_str(),
                            r.line,
                            "generic_param".to_string(),
                            pf.package_id,
                        ));
                        local_stats.external += 1;
                        continue;
                    }
                }

                // Tier 1.5: external classification. The same authority the
                // compiler-resolve reroute uses above — manifest/import hook,
                // chain-to-external, bare-name builtins, import list (guarded by
                // `is_module_in_project`), and module-qualified targets that name
                // no local file.
                let inferred_ns = classify_external_ns(
                    r,
                    &ref_ctx,
                    file_ctx,
                    file_imports,
                    &module_to_files,
                    &type_engine,
                    project_ctx,
                    index,
                    effective_lang,
                    // Tier-1.5 has no edge at stake — the chain heuristic is a
                    // reasonable last-resort classification here.
                    true,
                );

                if let Some(ns) = &inferred_ns {
                    // EXT-1 — scope-directed external routing. The classifier
                    // resolved this ref to a concrete external module (`ext:<mod>`).
                    // Record a module-scoped demand so the Stage-2 expand loop pulls
                    // the file that defines `target_name` *inside* that module and a
                    // re-resolve upgrades this opaque `external_ref` into a real edge.
                    // The pull is bounded: `SymbolLocationIndex::locate` only answers
                    // for (module, name) pairs the demand-driven index actually
                    // carries — builtin/primitive namespaces and modules the index
                    // never scanned locate to nothing and stay external_refs, exactly
                    // as today. The leaf of a dotted target (`Stripe.Event` → `Event`)
                    // is the name the package exports.
                    if let Some(module) = ns.strip_prefix("ext:").filter(|m| !m.is_empty()) {
                        let leaf = r
                            .target_name
                            .rsplit(['.', ':'])
                            .next()
                            .unwrap_or(r.target_name.as_str());
                        if !leaf.is_empty() {
                            index.record_chain_miss(leaf);
                        }
                    }
                    buf.externals.push((
                        source_id,
                        r.target_name.clone(),
                        r.kind.as_str(),
                        r.line,
                        ns.clone(),
                        pf.package_id,
                    ));
                    local_stats.external += 1;
                    continue;
                }

                // The heuristic Tier-2 fallback is gone. Every deterministic
                // strategy that lived in `heuristic.rs` was lifted into
                // `DefaultResolver` (engine tier) and called by every language
                // hook via `resolve_all()`. Refs reaching this point are
                // honestly unresolved.
                let resolution: Option<(i64, f64, &'static str)> = None;

                match resolution {
                    Some((target_id, confidence, strategy)) => {
                        buf.edges.push((
                            source_id,
                            target_id,
                            r.kind.as_str(),
                            r.line,
                            confidence,
                            strategy,
                        ));
                        local_stats.resolved += 1;
                    }
                    None => {
                        // Truly unresolved — no external namespace identified,
                        // no heuristic match found.
                        //
                        // Guard: the outer loop skips ext: files entirely, but a
                        // symbol's file_path could still be external (e.g. augmented
                        // from DB during incremental). Don't pollute unresolved_refs
                        // with gaps from third-party code — only project code's
                        // unresolved refs are the user's concern.
                        if pf.path.starts_with("ext:") {
                            continue;
                        }
                        // Imports edges point at a module, not a symbol. The
                        // heuristic can't bind them because the module name is
                        // a file stem rather than an identifier. Classify
                        // import edges generically:
                        //
                        //   * If the module name resolves to a project file
                        //     stem via `module_to_files`, the import is
                        //     satisfied locally — count as handled.
                        //   * If the leaf name appears in the SymbolIndex
                        //     under an `ext:` path, the import points at an
                        //     indexed external surface — count as handled.
                        //   * Otherwise the import points at a third-party
                        //     dependency the package manager didn't surface
                        //     (Nimble package not installed, Cabal package
                        //     not in the store). The dep is external by
                        //     definition; we just don't have its source.
                        //     Classify as external rather than unresolved so
                        //     "couldn't find symbol" stays distinct from
                        //     "import points at uninstalled dep".
                        if r.kind == EdgeKind::Imports {
                            let probe = r
                                .module
                                .as_deref()
                                .filter(|m| !m.is_empty())
                                .unwrap_or(r.target_name.as_str());
                            if is_module_in_project(probe, &module_to_files, index) {
                                local_stats.external += 1;
                                continue;
                            }
                            let leaf = probe.rsplit(['/', '.', ':']).next().unwrap_or(probe);
                            if !leaf.is_empty() {
                                let any_external = index
                                    .by_name(leaf)
                                    .iter()
                                    .any(|s| s.file_path.starts_with("ext:"));
                                if any_external {
                                    buf.externals.push((
                                        source_id,
                                        r.target_name.clone(),
                                        r.kind.as_str(),
                                        r.line,
                                        format!("ext:{probe}"),
                                        pf.package_id,
                                    ));
                                    local_stats.external += 1;
                                    continue;
                                }
                            }
                            // C/C++ `#include` directives name a header by its
                            // include-path (`windows.h`, `openssl/bio.h`), not a
                            // symbol. The path-keyed external header index
                            // (`build_c_header_index`) registers every SDK / vcpkg
                            // / POSIX header under that same include-path. Record an
                            // include-driven demand so the Stage-2 expand loop pulls
                            // the header via `SymbolLocationIndex::locate(path, path)`
                            // and a re-resolve admits its symbols. Both miss fields
                            // carry the include-path because the index keys headers
                            // at `(include_path, include_path)`. O(1) push — the pull
                            // happens demand-time in `expand`, never in this loop.
                            if is_c_family(effective_lang) && looks_like_header_include(probe) {
                                index.record_chain_miss(probe);
                            }
                            // Import we can't trace — write as unresolved so
                            // the ref stays visible to investigation queries
                            // rather than silently dropping it on the floor.
                            let from_snippet = pf
                                .symbol_from_snippet
                                .get(r.source_symbol_index)
                                .copied()
                                .unwrap_or(false);
                            buf.unresolved.push((
                                source_id,
                                r.target_name.clone(),
                                r.kind.as_str(),
                                r.line,
                                r.module.as_deref().map(|s| s.to_string()),
                                pf.package_id,
                                from_snippet,
                            ));
                            local_stats.unresolved += 1;
                            continue;
                        }
                        // Bare unresolved refs (no module path, no chain) record a
                        // miss so this file re-enters the frontier: a later pass
                        // probes `find_by_name(target)` against the external symbol
                        // index once inline materialization has pulled the declaring
                        // file. Without it, ambient identifiers declared in external
                        // `.d.ts` (Vue 3 macros, RxJS pipeable operators, lodash
                        // defaults) never get a second resolution attempt.
                        if r.module.is_none()
                            && r.chain.is_none()
                            && !r.target_name.is_empty()
                            && !r.target_name.contains('.')
                        {
                            index.record_chain_miss(&r.target_name);
                        }
                        let module_value = r.module.as_deref().map(|s| s.to_string());
                        // E3: propagate snippet flag from source symbol for
                        // aggregate-stats exclusion.
                        let from_snippet = pf
                            .symbol_from_snippet
                            .get(r.source_symbol_index)
                            .copied()
                            .unwrap_or(false);
                        buf.unresolved.push((
                            source_id,
                            r.target_name.clone(),
                            r.kind.as_str(),
                            r.line,
                            module_value,
                            pf.package_id,
                            from_snippet,
                        ));
                        local_stats.unresolved += 1;
                    }
                }
            }

            // Harvest bare-identifier return-type candidates the ref loop missed:
            // `return queryClient` / `return client` carry no ref (a param/local
            // read is not a cross-symbol reference), so `flow_return_lhs` never
            // saw them. Type the returned identifier against the function's
            // PARAMETERS — a typed param is emitted as a Property scoped to the
            // function, so its declared type lives in `field_type`. This is what
            // lets an arrow-const hook like `useQueryClient(qc?: QueryClient)
            // => { …; return qc }` infer `QueryClient`. (Typed locals are left
            // to a later pass; a single agreeing param candidate already infers.)
            for (fn_idx, ident) in &pf.flow.flow_return_ident {
                let Some(fn_sym) = pf.symbols.get(*fn_idx) else {
                    continue;
                };
                if fn_sym.return_type.is_some()
                    || index.return_type_name(&fn_sym.qualified_name).is_some()
                {
                    continue;
                }
                let Some(fn_db_id) = file_symbol_ids.get(*fn_idx).and_then(|id| *id) else {
                    continue;
                };
                let param_qname = format!("{}.{}", fn_sym.qualified_name, ident);
                if let Some(ty) = index.field_type_str(&param_qname) {
                    let is_generic_param = index
                        .generic_params(&fn_sym.qualified_name)
                        .map_or(false, |g| g.iter().any(|p| p == &ty));
                    if !ty.is_empty() && !ty.eq_ignore_ascii_case("unknown") && !is_generic_param {
                        buf.inferred_returns
                            .push((fn_sym.qualified_name.clone(), fn_db_id, ty));
                    }
                }
            }

            // R5: wipe the local-type cache so bindings from this file don't
            // leak into the next file processed on the same rayon worker.
            // (TLS cache survives across rayon tasks on the same worker
            // thread; explicit clear keeps it tight.)
            index.clear_local_cache();

            // Drain this file's chain misses (recorded into the worker-local
            // accumulator during the ref loop) into the per-file buffer. A file
            // with any miss joins the next pass's frontier.
            let file_misses = index.take_file_misses();
            if !file_misses.is_empty() {
                buf.frontier.push((pf.path.clone(), file_misses));
            }

            (buf, local_stats)
        })
        .reduce(
            || (FileWriteBuf::default(), FileStats::default()),
            |(mut buf_a, mut stats_a), (buf_b, stats_b)| {
                buf_a.merge(buf_b);
                stats_a.merge(stats_b);
                (buf_a, stats_a)
            },
        ));

    // The read-phase tx wrapped only DB reads (file imports). Close it, then
    // write the materialized external files under their own tx so their
    // symbol/file rows exist before the FK-enforced edges reference them, and
    // rebind edge targets from the in-pass synthetic ids to the real DB ids.
    tx.commit()
        .context("Failed to commit resolve read phase")?;
    let ext_remap = index.flush_materialized_externals(&*db)?;
    combined_buf.remap_edge_targets(&ext_remap);
    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin resolve write transaction")?;

    // Bulk-flush the per-file buffers in one transaction. Multi-row VALUES
    // inserts keep driver round-trips low; identical chunk sizes hit the rusqlite
    // stmt cache. When deferring (`defer_speculative.is_some()`) only edges are
    // persisted; the speculative rows are stashed below for one later flush.
    let persist_speculative = defer_speculative.is_none();
    flush_resolve_buf(&tx, &combined_buf, persist_speculative)?;
    if let Some(out) = defer_speculative {
        out.replace_from(&mut combined_buf);
    }
    stats.resolved += local_stats_total.resolved;
    stats.engine_resolved += local_stats_total.engine_resolved;
    stats.unresolved += local_stats_total.unresolved;
    stats.external += local_stats_total.external;

    // INFER-3: join return-type candidates per function (see
    // `join_inferred_returns`). Skip any qname still carrying a known return.
    if !combined_buf.inferred_returns.is_empty() {
        stats.inferred_returns = join_inferred_returns(&combined_buf.inferred_returns, |q| {
            index.return_type_name(q).is_some()
        });
    }

    tx.commit()
        .context("Failed to commit resolution transaction")?;

    // Flush resolver-emitted flow edges. Runs after the main transaction
    // so it can use the committed file rows for the file_id lookup.
    if !combined_buf.flow_emissions.is_empty() {
        let n = flush_flow_emissions(conn, &combined_buf.flow_emissions)?;
        if n > 0 {
            info!("Resolver-emitted flow edges: {n}");
        }
    }

    // `frontier_files` is the set of source paths that recorded any chain miss
    // this pass — the only files whose resolution can change once inline
    // externals materialization adds members, so the fixpoint re-resolves them.
    // Each file appears once (one per-file buffer per worker).
    stats.frontier_files = combined_buf.frontier.iter().map(|(p, _)| p.clone()).collect();

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

/// Decide the external namespace a ref belongs to, or `None`.
///
/// Authoritative signals only, in priority order: the language hook's
/// manifest/import classifier, chain-to-external inference, the bare-name
/// primitive/builtin tables, the file's import list (an imported name whose
/// module resolves to no local file, guarded by `is_module_in_project`), and
/// an explicit `r.module` that names no local file/namespace. A ref no
/// authority places in a dependency returns `None` — internal or honestly
/// unresolved, never branded external by elimination.
#[allow(clippy::too_many_arguments)]
fn classify_external_ns(
    r: &crate::types::ExtractedRef,
    ref_ctx: &RefContext,
    file_ctx: Option<&legacy::FileContext>,
    file_imports: &[(String, Option<String>)],
    module_to_files: &rustc_hash::FxHashMap<String, Vec<String>>,
    type_engine: &crate::type_checker::Engine,
    project_ctx: Option<&ProjectContext>,
    index: &SymbolIndex,
    effective_lang: &str,
    include_chain_heuristic: bool,
) -> Option<String> {
    file_ctx
        .and_then(|fc| type_engine.classify_external(ref_ctx, fc, project_ctx, index))
        // Chain-to-external: the chain walks to a type not in the index
        // (ORM, test framework, fluent API chains). This is a HEURISTIC — an
        // un-inferred chain root (a local variable whose type wasn't resolved)
        // looks the same as a genuine external receiver — so it is excluded
        // from the reroute decision, which must not override a grep edge on a
        // guess. Tier-1.5 (no edge at stake) still consults it.
        .or_else(|| {
            if !include_chain_heuristic {
                return None;
            }
            r.chain.as_ref().and_then(|chain| {
                legacy::infer_external_from_chain(chain, &ref_ctx.scope_chain, index)
            })
        })
        // Bare-name: test globals, language primitives, runtime builtins.
        // `effective_lang` so JS/TS refs embedded in Elixir/PHP/Ruby host
        // files classify against the JS/TS tables, not the host's.
        .or_else(|| {
            index
                .classify_external_name(&r.target_name, effective_lang)
                .map(|ns| ns.to_string())
        })
        // Profile-declared builtins: the `builtin_skip` predicate the resolver
        // uses to decline a name before the ladder marks language-core names
        // (Ada modular-type ops, proto well-known types) that the keyword-set
        // classifier above may not enumerate. Brand them builtin so a declined
        // name is an honest external, not a miscounted unresolved ref.
        .or_else(|| {
            type_engine
                .profile_for(effective_lang)
                .and_then(|p| p.builtin_skip)
                .filter(|is_builtin| is_builtin(r.target_name.as_str()))
                .map(|_| "builtin".to_string())
        })
        // Import-based: the ref's name (or its leading segment, for dotted
        // targets like `Stripe.Event`) matches an import whose module
        // resolves to no local file. Catches transitive bare-package deps,
        // slash-bearing sub-module specifiers (`rxjs/operators`), and
        // relative imports to build-time-generated files absent at scan.
        .or_else(|| {
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
                if is_module_in_project(module_path, module_to_files, index) {
                    continue;
                }
                return Some(format!("ext:{module_path}"));
            }
            None
        })
        // Module-qualified: `r.module` names no local file/namespace —
        // R `dplyr::mutate`, Erlang `lists:map`, Haskell `Map.lookup`.
        .or_else(|| {
            let module = r.module.as_ref()?;
            let mod_lower = module.to_lowercase();
            let mut is_local = module_to_files.contains_key(module.as_str())
                || module_to_files.contains_key(&mod_lower);
            // Last-segment match ONLY for single-segment modules — a
            // multi-segment `Ecto.Changeset` must not be called local just
            // because a `changeset.ex` file exists (coincidental stem match).
            if !is_local && !module.contains('.') {
                let last_seg = module.rsplit('.').next().unwrap_or(module);
                let last_lower = last_seg.to_lowercase();
                is_local = module_to_files.contains_key(last_seg)
                    || module_to_files.contains_key(&last_lower);
            }
            if is_local {
                None
            } else {
                Some(format!("ext:{module}"))
            }
        })
}

/// `true` when an external namespace string actually names a workspace-member
/// package — project-internal code, not a third-party dependency.
///
/// Strips an `ext:` prefix and any `.*` wildcard tail, then asks
/// `workspace_package_id` about the **full** remaining specifier first — its
/// own `/`-prefix walk matches a scoped package (`@scope/ui`) against a deep
/// import (`@scope/ui/button`). Falls back to the leading `::`/`.`-segment for
/// module paths whose package name is the first component (Rust/JVM).
fn names_workspace_package(ns: &str, lookup: &dyn SymbolLookup) -> bool {
    let probe = ns.strip_prefix("ext:").unwrap_or(ns);
    let probe = probe.strip_suffix(".*").unwrap_or(probe);
    if probe.is_empty() {
        return false;
    }
    if lookup.workspace_package_id(probe).is_some() {
        return true;
    }
    let first = probe
        .split(|c| c == ':' || c == '.')
        .find(|s| !s.is_empty())
        .unwrap_or(probe);
    first != probe && lookup.workspace_package_id(first).is_some()
}

/// Does the project's symbol index cover this import module?
///
/// Returns true if the module appears as a local namespace (any symbol has
/// that prefix) or maps to a local file via the heuristic module-to-file map.
/// A module that walks through multiple segments (`a.b.c`) is local when any
/// of those segments is covered — this prevents a false "external" classification
/// for package-qualified imports like Python `from app.core.db import engine`.
///
/// Relative specifiers (`./foo`, `../bar/Baz.astro`) are probed by the
/// trailing basename stem — `module_to_files` is keyed by stem, not by
/// fully-qualified path. Without this, every relative-path import of an
/// indexed file would appear external just because the map can't be queried
/// with the raw `../` form.
fn is_module_in_project(
    module_path: &str,
    module_to_files: &rustc_hash::FxHashMap<String, Vec<String>>,
    index: &legacy::SymbolIndex,
) -> bool {
    if module_to_files.contains_key(module_path) {
        return true;
    }
    let lower = module_path.to_lowercase();
    if lower != module_path && module_to_files.contains_key(&lower) {
        return true;
    }
    if index.has_in_namespace(module_path) {
        return true;
    }
    // Relative/slash-bearing path: probe by trailing basename.
    // `../../components/Aside.astro` → basename `Aside.astro` → stem `Aside`.
    // `./auth.service`               → basename `auth.service` (extension-less;
    //                                   `build_module_to_files` stores `auth.service`).
    // Try the full basename as-is (for extension-less module paths, where the
    // file `auth.service.ts` is keyed as `auth.service` in module_to_files)
    // AND the once-stripped stem (for paths carrying an explicit extension
    // like `Aside.astro`). Whichever matches first wins.
    if let Some(basename) = module_path.rsplit(['/', '\\']).next() {
        if basename != module_path && !basename.is_empty() {
            if module_to_files.contains_key(basename) {
                return true;
            }
            let basename_lower = basename.to_lowercase();
            if basename_lower != basename && module_to_files.contains_key(&basename_lower) {
                return true;
            }
            if let Some((stem, _)) = basename.rsplit_once('.') {
                if !stem.is_empty() {
                    if module_to_files.contains_key(stem) {
                        return true;
                    }
                    let stem_lower = stem.to_lowercase();
                    if stem_lower != stem && module_to_files.contains_key(&stem_lower) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// True for the C-family languages whose `#include` directives are admitted
/// through the path-keyed external header index.
fn is_c_family(language: &str) -> bool {
    matches!(language, "c" | "cpp")
}

/// True when an `#include` target names a header rather than a project-local
/// module. A header include is either a path ending in a header extension
/// (`stdio.h`, `openssl/bio.h`, `vector.hpp`) or an extensionless name with
/// no path separators — the extensionless C++ stdlib convention (`<vector>`,
/// `<memory>`). Path-bearing includes without a header extension
/// (`./local`, `../src/foo`) are project-relative and excluded so the demand
/// only fires for headers the path-keyed index can actually answer.
fn looks_like_header_include(include_path: &str) -> bool {
    if include_path.is_empty() {
        return false;
    }
    let basename = include_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(include_path);
    let is_header_ext = basename.ends_with(".h")
        || basename.ends_with(".hpp")
        || basename.ends_with(".hxx")
        || basename.ends_with(".hh");
    let is_extensionless_stdlib = !basename.contains('.') && !include_path.contains('/');
    is_header_ext || is_extensionless_stdlib
}

/// Pull the set of `origin='external'` file paths from the `files` table.
///
/// Used to seed the SymbolIndex so chain walkers can tell
/// script-tag-parsed vendor JS (`wwwroot/lib/jquery.min.js`, written with
/// `origin='external'` but keeping its regular project-relative path)
/// apart from user source. Without this signal, the internal filter in
/// `infer_external_from_chain` — which only checks for an `ext:` path
/// prefix — would count a vendored `$` / `jQuery` declaration as project
/// code and suppress external classification for every jQuery chain in
/// the user's own JS.
///
/// Returns an empty set on any SQL error; a missing external_paths set
/// degrades gracefully to the old `ext:`-prefix-only behaviour.
pub(super) fn read_external_file_paths(
    conn: &rusqlite::Connection,
) -> std::collections::HashSet<String> {
    let sql = "SELECT path FROM files WHERE origin = 'external'";
    let Ok(mut stmt) = conn.prepare_cached(sql) else {
        return std::collections::HashSet::new();
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
        return std::collections::HashSet::new();
    };
    rows.filter_map(|r| r.ok()).collect()
}

/// Pull a file's persisted `imports` rows directly from the DB and
/// rehydrate them as `ImportEntry` values. Used by the companion-import
/// merge when the companion file isn't in the current parse slice
/// (incremental re-index where only the template changed but the paired
/// component was already indexed in a prior run).
///
/// Returns an empty Vec on any SQL error; the path through here is best-
/// effort context enrichment, not a correctness-critical read.
fn read_file_imports_from_db(conn: &rusqlite::Connection, file_path: &str) -> Vec<ImportEntry> {
    let sql = "SELECT i.imported_name, i.module_path, i.alias
               FROM imports i
               JOIN files f ON f.id = i.file_id
               WHERE f.path = ?1";
    let Ok(mut stmt) = conn.prepare_cached(sql) else {
        return Vec::new();
    };
    let rows = stmt.query_map([file_path], |r| {
        let imported_name = r.get::<_, String>(0)?;
        Ok(ImportEntry {
            is_wildcard: imported_name == "*",
            imported_name,
            module_path: r.get::<_, Option<String>>(1)?,
            alias: r.get::<_, Option<String>>(2)?,
        })
    });
    match rows {
        Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
#[path = "loop_body_tests.rs"]
mod tests;
