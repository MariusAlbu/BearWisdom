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
use tracing::{debug, info};

use crate::connectors::url_pattern;
use crate::db::Database;
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

use super::adapters::{
    extracted_db_sets_to_emissions, extracted_routes_to_emissions,
    mailer_template_name_for_path, nextjs_route_consumer_emissions,
    plugin_flow_emissions_to_emissions,
};
use super::engine::{
    self, build_scope_chain, ChainMiss, ImportEntry, RefContext, ResolutionEngine, SymbolIndex,
    SymbolLookup,
};
use super::flow_emit;
use super::flow_pair::flush_flow_emissions;
use super::heuristic;
use super::indexes;
use super::write_buf::{flush_resolve_buf, FileStats, FileWriteBuf};
use super::ResolutionStats;

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

    resolve_iteration_body(db, parsed, symbol_id_map, project_ctx, &mut index, augmented_id_map)
}

pub(super) fn resolve_iteration_inner_with_index(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    index: &mut SymbolIndex,
) -> Result<ResolutionStats> {
    resolve_iteration_body(db, parsed, symbol_id_map, project_ctx, index, None)
}

fn resolve_iteration_body(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    index: &mut SymbolIndex,
    augmented_id_map: Option<HashMap<(String, String), i64>>,
) -> Result<ResolutionStats> {
    let engine = ResolutionEngine::new();
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
    let name_to_ids = indexes::build_name_index(merged_id_map_ref, parsed);
    let qname_to_id = indexes::build_qname_index(merged_id_map_ref);
    let module_to_files = indexes::build_module_to_files(parsed);
    let import_map = indexes::build_import_map(parsed);
    let file_namespace_map = indexes::build_file_namespace_map(parsed);

    // Phase 5: build the type-checker Engine alongside the legacy lookup
    // structures. Engine state is `Send + Sync` (TypeArena uses interior-
    // mutable RwLock) so a single shared `&Engine` drives the parallel
    // resolve closure below. We try engine.resolve as a fallback when the
    // per-language resolver returns None — purely additive, can only
    // improve resolution rates, never regress. Phase 6+ will swap the
    // ordering once we have confidence in engine output across more
    // languages.
    //
    // Convert the legacy `(path, qname) -> id` map into the engine's
    // `(path, idx) -> id` shape so SymbolTypeMap + MembersIndex can key
    // by canonical sym_id.
    let engine_sym_id_map: crate::type_checker::core::SymbolIdMap = {
        let mut map = crate::type_checker::core::SymbolIdMap::default();
        for pf in parsed {
            for (idx, sym) in pf.symbols.iter().enumerate() {
                if let Some(&id) = merged_id_map_ref.get(&(pf.path.clone(), sym.qualified_name.clone())) {
                    map.insert((pf.path.clone(), idx), id);
                }
            }
        }
        map
    };
    let type_engine = crate::type_checker::Engine::build_from_registry(
        parsed,
        &engine_sym_id_map,
        index,
        index.type_arena_arc(),
    );

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
            if pf.path.starts_with("ext:") { continue }
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
    let (mut combined_buf, local_stats_total) = parsed
        .par_iter()
        .filter(|pf| !pf.path.starts_with("ext:"))
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

        // Try language-specific resolver for this file.
        let host_resolver = engine.resolver_for(&pf.language);
        let host_plugin = crate::languages::default_registry().get_dedicated(&pf.language);
        let host_file_ctx = host_resolver.map(|r| {
            let mut ctx = type_engine
                .build_file_context(&pf.language, pf, project_ctx)
                .unwrap_or_else(|| r.build_file_context(pf, project_ctx));
            // Merge companion imports (e.g. Angular template inherits the
            // paired `.component.ts` imports, since the template itself has
            // no import statements but every symbol it names is imported by
            // the component class). Companion pairing lives on
            // `LanguagePlugin::companion_file_for_imports` — independent of
            // resolver wiring so any plugin can declare a paired file.
            if let Some(companion_path) =
                host_plugin.and_then(|p| p.companion_file_for_imports(&pf.path))
            {
                if let Some(comp_pf) = parsed_by_path.get(companion_path.as_str()) {
                    if let Some(comp_resolver) = engine.resolver_for(&comp_pf.language) {
                        let comp_ctx = type_engine
                            .build_file_context(&comp_pf.language, comp_pf, project_ctx)
                            .unwrap_or_else(|| comp_resolver.build_file_context(comp_pf, project_ctx));
                        ctx.imports.extend(comp_ctx.imports);
                    }
                } else if let Some(db_imports) = companion_db_imports.get(&companion_path) {
                    // Companion file isn't in this run's parse slice
                    // (incremental re-index, companion unchanged). The
                    // imports were prefetched into `companion_db_imports`
                    // before the parallel section so the per-file body
                    // doesn't need `conn`.
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
        index.install_local_cache(narrowings);

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
        let uses_flow = !pf.flow.flow_binding_lhs.is_empty()
            || !pf.flow.narrowings.is_empty();
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
            let (resolver, file_ctx): (Option<&dyn engine::LanguageResolver>, _) =
                if is_cross_lang_embedded {
                    let emb_resolver = engine.resolver_for(effective_lang);
                    let emb_ctx = emb_resolver.map(|res| {
                        type_engine
                            .build_file_context(effective_lang, pf, project_ctx)
                            .unwrap_or_else(|| res.build_file_context(pf, project_ctx))
                    });
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
                local_stats.external += 1; // count as "handled" so they don't inflate unresolved rate
                continue;
            }

            // R5: move the flow-cache cursor to this ref's byte offset so
            // narrowing lookups in chain walkers see the right scope.
            // Sprint 1 leaves this as 0 for languages that haven't wired
            // their FlowConfig yet — narrowings are empty in that case so
            // the cursor value doesn't matter.
            let ref_byte = pf
                .flow
                .ref_byte_offsets
                .get(ref_idx)
                .copied()
                .unwrap_or(0);
            index.set_cursor(ref_byte);

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

                // Flow-emission detection runs regardless of whether resolution
                // succeeds — HTTP client calls, IPC, WebSocket emits, etc. are
                // identifiable from import context alone, even when the chain
                // walker can't resolve the external symbol to a DB id.
                for emission in type_engine.detect_flow_emissions(file_ctx, &ref_ctx, index) {
                    buf.flow_emissions.push((pf.path.clone(), r.line, emission));
                }

                // Languages with a registered profile AND opted-in via
                // `LanguageProfile::engine_primary` route through the engine
                // first for every ref shape. Chain-bearing refs walk the
                // engine's unified chain walker; chain-less refs run the
                // engine's bare-name resolver. The legacy `LanguageResolver`
                // still runs as fallback when the engine declines (`None`),
                // covering language-specific behaviors the engine hasn't
                // adopted yet (TS workspace packages, tsconfig path aliases,
                // DefinitelyTyped fallback, barrel re-exports).
                let try_engine_first = type_engine
                    .profile_for(&pf.language)
                    .map(|p| p.engine_primary)
                    .unwrap_or(false);
                let hook_resolve = || {
                    type_engine
                        .resolve_ref_via_hook(effective_lang, file_ctx, &ref_ctx, index)
                        .map(|r| (r, false))
                };
                let resolution = if try_engine_first {
                    type_engine
                        .resolve(&ref_ctx, file_ctx, index)
                        .map(|r| (r, true))
                        .or_else(hook_resolve)
                } else {
                    hook_resolve()
                };

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
                                            .or_else(|| {
                                                index.field_type_str(&s.qualified_name)
                                            })
                                    })
                            });
                        if let Some(yield_str) = yield_str {
                            if let Some(lhs_sym) = pf.symbols.get(lhs_idx) {
                                index.record_local_type(
                                    lhs_sym.name.clone(),
                                    yield_str,
                                );
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
                // Walk the source symbol's own qualified name first, then up
                // through its parents. The function/struct itself owns its
                // type parameters; refs in its signature have its qname (not
                // its parent) as the relevant scope for generic-param lookup.
                let is_generic_param = std::iter::once(source_sym.qualified_name.as_str())
                    .chain(scope_chain.iter().map(String::as_str))
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

            // Tier 1.5: external classification — `LanguageEngineHooks::classify_external`
            // is the only entry point. The legacy resolver method has been retired.
            let inferred_ns = if let (Some(_resolver), Some(file_ctx)) = (resolver, &file_ctx) {
                let ref_ctx = RefContext {
                    extracted_ref: r,
                    source_symbol: source_sym,
                    scope_chain: scope_chain.clone(),
                    file_package_id: pf.package_id,
                };
                type_engine.classify_external(&ref_ctx, file_ctx, project_ctx, index)
            } else {
                None
            };

            // Chain-to-external: if the chain walks to a type not in the index,
            // classify as external (handles ORM, test framework, fluent API chains).
            let inferred_ns = inferred_ns.or_else(|| {
                r.chain.as_ref().and_then(|chain| {
                    engine::infer_external_from_chain(chain, &scope_chain, index)
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
            // Catches three practical cases the language-specific manifest
            // checks miss:
            //   1. Transitive bare-package deps — e.g. Java `import
            //      tools.jackson.databind.ObjectMapper` (Jackson 3.x, pulled in
            //      via spring-boot-starter, not declared in pom.xml) or Python
            //      `from sqlalchemy import Engine` (transitive of sqlmodel).
            //   2. Bare-package deep imports like `rxjs/operators`,
            //      `lodash/fp`, `date-fns/utcToZonedTime` — the slash-bearing
            //      specifier isn't a relative path, it's a sub-module of an
            //      indexed package that the manifest may not enumerate.
            //   3. Relative imports to files that don't exist in the index —
            //      e.g. NSwag-generated `'../web-api-client'` that's produced
            //      at build time and absent at scan time.
            //
            // Language-agnostic: relies only on the project's own symbol
            // index via `is_module_in_project` as the sole "is this actually
            // local?" authority. Imports whose target the module resolvers
            // couldn't reach, by any path, are called external — honest to
            // "we don't have its definition" without inventing a fake edge.
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
                    if is_module_in_project(module_path, &module_to_files, index) {
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
                &|p| index.is_ambient_path(p),
                &|suffix, prefix, module, cands| {
                    index.resolve_via_external_reexport(suffix, prefix, module, cands)
                },
            );

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
                                local_stats.external += 1;
                                continue;
                            }
                        }
                        // Uninstalled / unwalked third-party dep. The
                        // import is real, the source just isn't on disk.
                        local_stats.external += 1;
                        continue;
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

            // R5: wipe the local-type cache so bindings from this file don't
            // leak into the next file processed on the same rayon worker.
            // (TLS cache survives across rayon tasks on the same worker
            // thread; explicit clear keeps it tight.)
            index.clear_local_cache();

            (buf, local_stats)
        })
        .reduce(
            || (FileWriteBuf::default(), FileStats::default()),
            |(mut buf_a, mut stats_a), (buf_b, stats_b)| {
                buf_a.merge(buf_b);
                stats_a.merge(stats_b);
                (buf_a, stats_a)
            },
        );

    // Bulk-flush the per-file buffers in one transaction. Multi-row
    // VALUES inserts cut driver round-trips ~Nx vs the previous per-ref
    // path. Identical chunk sizes hit the rusqlite stmt cache.
    flush_resolve_buf(&tx, &combined_buf)?;
    stats.resolved += local_stats_total.resolved;
    stats.engine_resolved += local_stats_total.engine_resolved;
    stats.unresolved += local_stats_total.unresolved;
    stats.external += local_stats_total.external;

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

    // Drain chain walker bail-outs for the orchestrator's R3 reload pass.
    // Deduped on (current_type, target_name) — the second pass cares about
    // unique misses; the source-ref retry list is recovered from the DB's
    // `unresolved_refs` table.
    let raw_misses = index.take_chain_misses();
    let mut seen: std::collections::HashSet<ChainMiss> = std::collections::HashSet::new();
    let mut unique_misses: Vec<ChainMiss> = Vec::new();
    for m in raw_misses.iter().cloned() {
        if seen.insert(m.clone()) { unique_misses.push(m); }
    }
    if !unique_misses.is_empty() {
        debug!(
            "Chain walker recorded {} bail-outs ({} unique)",
            raw_misses.len(),
            unique_misses.len(),
        );
    }
    stats.chain_misses = unique_misses;

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
    index: &engine::SymbolIndex,
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
            if basename_lower != basename
                && module_to_files.contains_key(&basename_lower)
            {
                return true;
            }
            if let Some((stem, _)) = basename.rsplit_once('.') {
                if !stem.is_empty() {
                    if module_to_files.contains_key(stem) {
                        return true;
                    }
                    let stem_lower = stem.to_lowercase();
                    if stem_lower != stem
                        && module_to_files.contains_key(&stem_lower)
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
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
fn read_file_imports_from_db(
    conn: &rusqlite::Connection,
    file_path: &str,
) -> Vec<ImportEntry> {
    let sql = "SELECT i.imported_name, i.module_path, i.alias
               FROM imports i
               JOIN files f ON f.id = i.file_id
               WHERE f.path = ?1";
    let Ok(mut stmt) = conn.prepare_cached(sql) else {
        return Vec::new();
    };
    let rows = stmt.query_map([file_path], |r| {
        Ok(ImportEntry {
            imported_name: r.get::<_, String>(0)?,
            module_path: r.get::<_, Option<String>>(1)?,
            alias: r.get::<_, Option<String>>(2)?,
            is_wildcard: false, // Not persisted; safe default.
        })
    });
    match rows {
        Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
        Err(_) => Vec::new(),
    }
}
