// =============================================================================
// engine/pipeline.rs — single-pass resolution entry for the new engine
//
// Builds a `Compilation` from parse output, materializes the externals the
// project reaches INTO that same tree, resolves every internal ref exactly once
// through `SemanticModel`, and bulk-writes edges + unresolved_refs to the DB. One
// pass, no fixpoint, no old-engine resolution code — externals are resolved like
// internals, the only difference being they are pulled from disk on first sight.
// =============================================================================

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use rustc_hash::{FxHashMap, FxHashSet};

pub(super) use super::async_yield::unwrap_async_yield_id;
use super::async_yield::unwrap_async_yield_str;
use super::file_context::{self, build_file_context, build_plugin_lookup, build_profiles};
use super::file_lookup::FileLookup;
use super::flush::{flush_to_db, Edge, RefLog, Unresolved};
use super::occurrence_census::FileCensus;
use crate::occurrence::Disposition;

use crate::db::Database;
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::indexer::flow_assignments::TUPLE_INDEX_KEY_PREFIX;
use crate::indexer::plugin_state::PluginStateBag;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::cause::CauseKind;
use crate::indexer::resolve::engine::contract::{FlowCacheLookup, RefContext, SymbolLookup};
use crate::indexer::resolve::engine::trace;
use crate::indexer::resolve::engine::{
    compilation::Compilation,
    semantic_model::{SemanticModel, SolveOutcome},
};
use crate::indexer::resolve::ResolutionStats;
use crate::indexer::write::SymbolIds;
use crate::languages::LanguagePlugin;
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{EdgeKind, ParsedFile};

use crate::indexer::resolve::engine::contract::build_scope_chain;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

// Tree construction lives in `tree_build`; re-exported so callers keep the
// established `engine::pipeline::*` path.
pub use super::tree_build::{materialize_and_build_tree, rebuild_tree};

/// Resolve every internal file in `parsed` against an already-built `tree`
/// and bulk-write the resulting edges + unresolved_refs to the DB (replacing
/// whatever was there before). Tail half of the single-pass entry point,
/// split so a caller can refresh plugin state and rebuild `tree` in between.
pub fn resolve_from_tree(
    db: &mut Database,
    mut tree: Compilation,
    parsed: &[ParsedFile],
    symbol_id_map: &SymbolIds,
    project_ctx: Option<&ProjectContext>,
) -> Result<ResolutionStats> {
    let profiles = build_profiles();
    let plugins = build_plugin_lookup();
    let plugin_state = project_ctx.map(|c| &c.plugin_state);

    super::inference_prelude::run(&mut tree, parsed, symbol_id_map, &profiles);

    let solver = SemanticModel::production();

    // Inherits pre-pass: bind Inherits/Implements refs first and merge their
    // RESOLVED (child, parent) ids into the inheritance map, so every member
    // walk in the main sweep climbs the parents resolution actually chose —
    // identity, never string re-derivation.
    let (pre_edges, _, _, _) = {
        let _t = crate::indexer::phase_timer::scope("resolve.inherits_prepass");
        super::parallel_pass::run(
            parsed,
            &tree,
            &profiles,
            &plugins,
            plugin_state,
            &solver,
            symbol_id_map,
            Some(super::parallel_pass::INHERIT_KINDS),
        )
    };
    tree.apply_resolved_inherits(pre_edges.iter().map(|e| (e.0, e.1)));

    let (edges, unresolved, ref_log, censuses) = {
        let _t = crate::indexer::phase_timer::scope("resolve.main_sweep");
        super::parallel_pass::run(
            parsed,
            &tree,
            &profiles,
            &plugins,
            plugin_state,
            &solver,
            symbol_id_map,
            None,
        )
    };

    let mut stats = ResolutionStats::default();
    stats.resolved = edges.len() as u64;
    stats.unresolved = unresolved.len() as u64;

    // Write: replace all four resolution tables atomically.
    {
        let _t = crate::indexer::phase_timer::scope("resolve.flush");
        flush_to_db(db, &edges, &unresolved, &ref_log, &censuses, true)?;
    }
    // Persist the resolved type metadata so an incremental pass can load it back
    // exactly, instead of re-deriving a lossier version from signatures alone.
    let _t = crate::indexer::phase_timer::scope("resolve.persist_type_info");
    tree.persist_type_info(db.conn())
        .context("Failed to persist symbol type info")?;
    drop(_t);

    Ok(stats)
}

/// Single-pass resolution: `materialize_and_build_tree` then
/// `resolve_from_tree`, no plugin-state refresh point in between. Callers
/// that refresh plugin cross-file state against a demand-pulled external
/// batch (see `indexer::plugin_state_phase`) call the two halves directly
/// instead — `full_index` does this.
///
/// `project_ctx` supplies the generic project data the engine resolves against
/// (workspace-package names); the `Compilation` snapshots only those generic
/// fields, never language- or ecosystem-specific state.
pub fn resolve_single_pass(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &SymbolIds,
    project_ctx: Option<&ProjectContext>,
    arena: Arc<TypeArena>,
    loc: Arc<SymbolLocationIndex>,
) -> Result<ResolutionStats> {
    let (tree, _ext_parsed, _ext_id_map) =
        materialize_and_build_tree(db, parsed, symbol_id_map, project_ctx, arena, loc)?;
    resolve_from_tree(db, tree, parsed, symbol_id_map, project_ctx)
}

/// Incremental resolution for the rule-based engine — the complement of
/// `resolve_single_pass`. Builds a `Compilation` from the changed files in
/// `parsed`, loads the unchanged remainder (and the externals the last full
/// index materialized) plus their persisted type metadata from the DB, resolves
/// the changed files' refs, and INSERTS their edges / unresolved rows (the old
/// rows were dropped upstream when the changed files' symbols were rewritten).
///
/// No on-disk externals walk runs here: externals don't change on a source edit,
/// so they are read back from the DB via `ingest_from_db`.
pub fn resolve_incremental_pass(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &SymbolIds,
    project_ctx: Option<&ProjectContext>,
    arena: Arc<TypeArena>,
) -> Result<ResolutionStats> {
    let ambient_qnames = crate::ecosystem::ambient::ambient_global_qnames(parsed);
    let mut tree = Compilation::build_with_context(
        parsed,
        symbol_id_map,
        Arc::clone(&arena),
        project_ctx,
        &ambient_qnames,
    );

    // Grow the tree with every other symbol in the index — unchanged internal
    // files plus the externals a prior full index already materialized — and
    // their persisted type metadata. The returned id map covers every DB symbol;
    // fold it into the changed-files map so an affected file's source symbols
    // (which live only in the DB) map to their ids during resolution.
    let mut full_id_map = symbol_id_map.clone();
    for ((path, qname), v) in tree.ingest_from_db(db.conn()) {
        full_id_map.insert_key_if_absent(path, qname, v);
    }

    // Built before the prelude so `infer_field_init_types` can read each
    // file's `async_wrappers`. Wrapped functions are loaded from the DB at
    // this point, so the prelude's wrapper pass reads complete state.
    let profiles = build_profiles();
    let plugins = build_plugin_lookup();
    let plugin_state = project_ctx.map(|c| &c.plugin_state);
    super::inference_prelude::run(&mut tree, parsed, symbol_id_map, &profiles);

    let solver = SemanticModel::production();

    let (pre_edges, _, _, _) = super::parallel_pass::run(
        parsed,
        &tree,
        &profiles,
        &plugins,
        plugin_state,
        &solver,
        &full_id_map,
        Some(super::parallel_pass::INHERIT_KINDS),
    );
    tree.apply_resolved_inherits(pre_edges.iter().map(|e| (e.0, e.1)));

    let (edges, unresolved, ref_log, censuses) = super::parallel_pass::run(
        parsed,
        &tree,
        &profiles,
        &plugins,
        plugin_state,
        &solver,
        &full_id_map,
        None,
    );

    let mut stats = ResolutionStats::default();
    stats.resolved = edges.len() as u64;
    stats.unresolved = unresolved.len() as u64;

    // Insert-only: do NOT clear the tables — only the changed files' rows were
    // dropped upstream; everything else must survive.
    flush_to_db(db, &edges, &unresolved, &ref_log, &censuses, false)?;
    tree.persist_type_info(db.conn())
        .context("Failed to persist incremental type and module metadata")?;

    Ok(stats)
}

/// Resolve one internal file's refs, returning its edges and unresolved rows.
/// Refs are visited in source order so a later ref sees the flow-inferred type
/// of a local bound by an earlier ref. No shared mutable state — files run
/// concurrently over the read-only `tree`. `only_kinds` narrows the sweep to
/// a ref-kind subset (the inherits pre-pass); `None` visits every ref.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_one_file<'a>(
    pf: &'a ParsedFile,
    tree: &Compilation,
    profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
    plugins: &FxHashMap<&'static str, &'static dyn LanguagePlugin>,
    plugin_state: Option<&PluginStateBag>,
    solver: &SemanticModel,
    symbol_id_map: &SymbolIds,
    only_kinds: Option<&[EdgeKind]>,
) -> (Vec<Edge>, Vec<Unresolved>, Vec<RefLog>, FileCensus<'a>) {
    let mut edges: Vec<Edge> = Vec::new();
    let mut unresolved: Vec<Unresolved> = Vec::new();
    let mut ref_log: Vec<RefLog> = Vec::new();
    let mut census = FileCensus::new(pf);

    let Some(&profile) = profiles.get(pf.language.as_str()) else {
        for (i, r) in pf.refs.iter().enumerate() {
            if only_kinds.is_none_or(|kinds| kinds.contains(&r.kind)) {
                census.record(i, Disposition::UnsupportedLanguage);
            }
        }
        return (edges, unresolved, ref_log, census);
    };

    let plugin = plugins.get(pf.language.as_str()).copied();
    let file_ctx = build_file_context(&pf.language, pf, profile, plugin, plugin_state);

    // Fresh per-file flow cache: local bindings from earlier refs in this file
    // are visible to later refs in the same file only.
    let file_lookup = FileLookup::for_file(tree, pf, symbol_id_map);

    // Migrated files already seed declarations by BindingId in LexicalCache.
    // Unmigrated files retain deterministic legacy name seeding; ordering avoids
    // HashMap nondeterminism but does not fix their same-name scope collisions.
    if pf.flow.lexical.is_none() {
        let mut decl_seeds: Vec<_> = pf.flow.flow_binding_decl_type.iter().collect();
        decl_seeds.sort_unstable_by_key(|&(idx, _)| *idx);
        for (&lhs_idx, decl_ty) in decl_seeds {
            if let Some(sym) = pf.symbols.get(lhs_idx) {
                file_lookup.record_local_type(sym.name.clone(), decl_ty.clone());
            }
        }
    }

    // Snapshot the trace filters once per file, outside the per-ref loop.
    // The relaxed load is the zero-cost gate; the filter read (Mutex) only
    // happens when TRACE_ACTIVE is true. A single index pass traces every ref
    // matching any filter in the set.
    let trace_filters = if trace::TRACE_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
        trace::get_filters()
    } else {
        Vec::new()
    };

    // Extractors may emit the same node more than once (double-visited
    // constructs, per-node-kind coverage double-emits). A duplicate re-runs
    // the full strategy ladder for an identical outcome and lands on the same
    // UNIQUE row, so only the first occurrence of a ref site is resolved.
    // byte_offset keeps distinct same-name refs on one line separate.
    let mut seen_sites = FxHashSet::default();

    let mut ordered_refs: Vec<_> = pf.refs.iter().enumerate().collect();
    if pf.flow.lexical.is_some() {
        ordered_refs.sort_by_key(|(_, r)| {
            (
                r.byte_offset,
                r.chain
                    .as_ref()
                    .and_then(|c| c.segments.last())
                    .map(|s| s.byte_offset)
                    .unwrap_or(r.byte_offset),
            )
        });
    }
    for (ref_idx, r) in ordered_refs {
        file_lookup.set_cursor(r.byte_offset);
        if only_kinds.is_some_and(|ks| !ks.contains(&r.kind)) {
            continue;
        }
        if !seen_sites.insert((
            r.source_symbol_index,
            r.kind,
            if pf.flow.lexical.is_some() && r.kind == EdgeKind::Calls {
                None // Syntax-addressed calls do not use spelling as identity.
            } else {
                Some(r.target_name.as_str())
            },
            r.line,
            r.byte_offset,
            r.chain
                .as_ref()
                .and_then(|c| c.segments.last())
                .map(|s| s.byte_offset),
        )) {
            census.record(ref_idx, Disposition::Duplicate);
            continue;
        }
        let Some(source_sym) = pf.symbols.get(r.source_symbol_index) else {
            census.record(ref_idx, Disposition::MissingSourceSymbol);
            continue;
        };
        let Some(source_id) =
            symbol_id_map.id_of(&pf.path, r.source_symbol_index, &source_sym.qualified_name)
        else {
            census.record(ref_idx, Disposition::MissingSourceId);
            continue;
        };

        // Synthetic primitive-type marker emitted by the extractor — not a
        // resolvable symbol, so it is neither an edge nor an unresolved ref.
        if r.target_name == "_primitive" {
            census.record(ref_idx, Disposition::Primitive);
            continue;
        }

        // Propagate the source symbol's snippet flag so unresolved rows from
        // Markdown fence code, Rust doctests, and Python doctests carry
        // from_snippet=true. The CODE_REF_FILTER in query/stats.rs excludes
        // such rows from resolution-rate aggregates — snippets are sample code
        // that typically lacks imports, so their unresolved refs are expected
        // and must not drag the project's resolution rate down.
        let ref_is_snippet = pf
            .symbol_from_snippet
            .get(r.source_symbol_index)
            .copied()
            .unwrap_or(false);

        let ref_ctx = RefContext {
            extracted_ref: r,
            source_symbol: source_sym,
            scope_chain: build_scope_chain(source_sym.scope_path.as_deref()),
            file_package_id: pf.package_id,
            source_symbol_id: Some(source_id),
        };

        let kind_str = edge_kind_str(r.kind);

        // Activate per-ref tracing when any filter matches this file + line + target.
        // Ref lines are 0-based tree-sitter rows; accept the editor's 1-based line
        // too so either convention matches.
        if trace_filters
            .iter()
            .any(|(suffix, filter_line, filter_target)| {
                pf.path.ends_with(suffix.as_str())
                    && (r.line == *filter_line || r.line + 1 == *filter_line)
                    && (filter_target.is_empty() || r.target_name == *filter_target)
            })
        {
            // Install the collector first so the REF header lands in it.
            trace::begin_ref();
            let chain_desc = r.chain.as_ref().map(|c| {
                c.segments
                    .iter()
                    .map(|s| {
                        if s.is_call {
                            format!("{}(call)", s.name)
                        } else {
                            s.name.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(".")
            });
            crate::tracef!(
                "REF target='{}' kind={} line={} source='{}' chain=[{}]",
                r.target_name,
                kind_str,
                r.line,
                source_sym.qualified_name,
                chain_desc.as_deref().unwrap_or(""),
            );
        }

        match solver.get_symbol_info(&ref_ctx, &file_ctx, &file_lookup, profile) {
            SolveOutcome::Resolved(mut res) => {
                census.record(ref_idx, Disposition::Resolved);
                // A chain-less call (`const x = makeThing(user)`) never enters the
                // chain walker — it needs a receiver segment plus a member, so a
                // call with fewer than two segments is bound by the bare ladder
                // and nothing has applied its arguments to the callee's generics
                // yet. Do it here against the resolved callee, filling the
                // return's open parameters from the argument types.
                let bare_call = r.chain.as_ref().is_none_or(|c| c.segments.len() < 2);
                // Navigation binds the lexical declaration; existing callable
                // provenance is a separate ID used for yield and graph edges.
                let callee_id = (bare_call && r.kind == EdgeKind::Calls)
                    .then(|| {
                        file_lookup
                            .local_reference(r.byte_offset)
                            .and_then(|local| local.callable)
                    })
                    .flatten()
                    .unwrap_or(res.target_symbol_id);
                if callee_id != res.target_symbol_id {
                    res.resolved_yield_type = res
                        .resolved_yield_type
                        .or_else(|| tree.return_type_id_of(callee_id));
                }
                if bare_call && matches!(r.kind, EdgeKind::Calls | EdgeKind::Instantiates) {
                    if let (Some(arena), Some(callee)) =
                        (tree.type_arena(), tree.symbol_by_id(callee_id))
                    {
                        let yielded = res
                            .resolved_yield_type
                            .or_else(|| tree.return_type_id_of(callee_id));
                        res.resolved_yield_type = super::chain::apply_call_args(
                            &file_lookup,
                            arena,
                            callee,
                            r.byte_offset,
                            &r.call_args,
                            &file_lookup
                                .local_reference(r.byte_offset)
                                .map(|local| local.type_args)
                                .unwrap_or_default(),
                            arena.intern(crate::type_checker::core::types::Type::Unknown),
                            None,
                            yielded,
                            profile.delegate_wrappers,
                        );
                    }
                }
                // Forward inference: when this ref is the RHS of a local binding,
                // record the yield type so a later ref rooted on the same variable
                // name can walk the chain.
                //
                // The TypeId path is preferred: `resolved_yield_type` is already a
                // canonical TypeId that correctly represents primitives, optionals,
                // and generics. Storing it directly via `record_local_type_id`
                // avoids the `format_type` → `intern_type_str` round-trip that
                // nominalizes those types to `Class`. The String path is kept as a
                // fallback for bindings that carry no `resolved_yield_type` (the
                // `return_type_str` / `field_type_str` / `Instantiates` branches).
                let target_qname = tree
                    .by_name(&r.target_name)
                    .iter()
                    .find(|s| s.id == res.target_symbol_id)
                    .map(|s| s.qualified_name.clone())
                    .unwrap_or_else(|| format!("id={}", res.target_symbol_id));
                let yield_type_fmt = res
                    .resolved_yield_type
                    .map(|id| format!("{id:?}"))
                    .unwrap_or_else(|| "None".to_string());
                crate::tracef!(
                    "RESULT resolved -> '{}' strategy={} yield_type={}",
                    target_qname,
                    res.strategy,
                    yield_type_fmt,
                );

                if let Some(&lhs_idx) = pf.flow.flow_binding_lhs.get(&ref_idx) {
                    if let Some(lhs_sym) = pf.symbols.get(lhs_idx) {
                        let is_awaited = pf.flow.flow_binding_await.contains(&lhs_idx);
                        if let Some(yield_id) = res.resolved_yield_type {
                            // TypeId available: store it directly. No format/intern.
                            // When the binding was awaited, strip one async-wrapper
                            // layer so `Promise<T>` → `T` before recording.
                            let final_id = if is_awaited {
                                if let Some(arena) = tree.type_arena() {
                                    unwrap_async_yield_id(yield_id, arena, profile.async_wrappers)
                                } else {
                                    yield_id
                                }
                            } else {
                                yield_id
                            };
                            crate::tracef!(
                                "SEED bound_lhs='{}' yield_type_id={:?} awaited={} -> recorded=TypeId",
                                lhs_sym.name,
                                final_id,
                                is_awaited,
                            );
                            file_lookup.record_rhs_type(ref_idx, &lhs_sym.name, final_id);
                        } else {
                            // No TypeId from the resolver — derive one from the target
                            // symbol's id-keyed type metadata. Recover the symbol by id
                            // (not a by_name scan) and read its type by id (not a qname
                            // string), so a callee whose qname is shared across packages
                            // seeds THIS declaration's type, and record the TypeId so the
                            // binding keeps identity rather than nominalizing to a string.
                            let target_id = res.target_symbol_id;
                            // A call / construction yields the callee's return type (a
                            // class's own return is itself); any other binding RHS yields
                            // the value's field type.
                            let yield_id = match r.kind {
                                EdgeKind::Calls | EdgeKind::Instantiates => {
                                    tree.return_type_id_of(target_id)
                                }
                                _ => tree.field_type_id_of(target_id),
                            };
                            if let Some(id) = yield_id {
                                let final_id = if is_awaited {
                                    if let Some(arena) = tree.type_arena() {
                                        unwrap_async_yield_id(id, arena, profile.async_wrappers)
                                    } else {
                                        id
                                    }
                                } else {
                                    id
                                };
                                crate::tracef!(
                                    "SEED bound_lhs='{}' meta_type_id={:?} awaited={} -> recorded=TypeId",
                                    lhs_sym.name,
                                    final_id,
                                    is_awaited,
                                );
                                file_lookup.record_rhs_type(ref_idx, &lhs_sym.name, final_id);
                            } else {
                                // Id-keyed metadata absent (id-less external, or a class
                                // whose return slot the index didn't populate) — fall
                                // back to the qname-string type, the symbol recovered by id.
                                let yield_ty = tree.symbol_by_id(target_id).and_then(|s| {
                                    match r.kind {
                                        // `const x = f()` — x is f's return type.
                                        EdgeKind::Calls => tree.return_type_str(&s.qualified_name),
                                        // `const x = new Foo()` — x IS Foo.
                                        EdgeKind::Instantiates => Some(s.qualified_name.clone()),
                                        _ => tree.field_type_str(&s.qualified_name),
                                    }
                                });
                                // When the binding was awaited, strip one async-wrapper
                                // layer from the string type before recording.
                                let final_ty = yield_ty.as_deref().and_then(|ty| {
                                    if is_awaited {
                                        unwrap_async_yield_str(ty, profile.async_wrappers)
                                            .map(|s| s.to_string())
                                            .or_else(|| Some(ty.to_string()))
                                    } else {
                                        Some(ty.to_string())
                                    }
                                });
                                crate::tracef!(
                                    "SEED bound_lhs='{}' yield_type_id=None awaited={} string_fallback={} -> recorded={}",
                                    lhs_sym.name,
                                    is_awaited,
                                    final_ty.as_deref().unwrap_or("None"),
                                    if final_ty.is_some() { "String" } else { "nothing" },
                                );
                                if let Some(ty) = final_ty {
                                    file_lookup.record_rhs_text(ref_idx, &lhs_sym.name, ty);
                                } else {
                                    // Nothing seeded this binding's type at all —
                                    // `target_id` (the initializer's callee/value)
                                    // resolved but its OWN return/field type was
                                    // never captured. Record that as the cause a
                                    // later ref rooted on `lhs_sym.name` will read,
                                    // so the diagnosis blames the initializer, not
                                    // this binding.
                                    let cause_kind = if matches!(r.kind, EdgeKind::Calls) {
                                        CauseKind::UncapturedReturn
                                    } else {
                                        CauseKind::UncapturedField
                                    };
                                    file_lookup.record_rhs_cause(
                                        ref_idx,
                                        &lhs_sym.name,
                                        crate::indexer::resolve::engine::cause::Cause::new(
                                            Some(target_id),
                                            cause_kind,
                                        ),
                                    );
                                }
                            }
                        }
                    }
                } else {
                    crate::tracef!("SEED none (ref is not a binding RHS)");
                }

                // Destructured bindings of this RHS: object fields (`const { a,
                // b: c } = f()`) or flat tuple elements (`const [a, b] = f()`).
                // Object bindings project a field on the call's yield type R;
                // tuple bindings project only a direct `Type::Tuple` element.
                if let Some(entries) = pf.flow.flow_binding_destructure.get(&ref_idx) {
                    if let Some(arena) = tree.type_arena() {
                        // Explicit call type args (`useQuery<Movie>()`), interned for
                        // binding into the callee's return.
                        let call_arg_ids: Vec<TypeId> = r
                            .chain
                            .as_ref()
                            .and_then(|c| c.segments.last())
                            .map(|s| {
                                s.type_args
                                    .iter()
                                    .map(|t| arena.intern_type_str(t))
                                    .collect()
                            })
                            .unwrap_or_default();
                        // When the call carries explicit type args (`useQuery<Movie>()`),
                        // bind them into the import-scoped callee's return FIRST — the
                        // substituted result types a destructured field concretely. This
                        // must precede the stored-return slot: `set_return_both` copies a
                        // callee's return onto every same-qname id (including the re-export
                        // barrel), so the unsubstituted return would otherwise win and the
                        // field would type to a bare generic param.
                        let recv_ty = (if !call_arg_ids.is_empty()
                            && matches!(r.kind, EdgeKind::Calls | EdgeKind::Instantiates)
                        {
                            crate::indexer::resolve::engine::chain::call_return_with_type_args(
                                &file_lookup,
                                arena,
                                &file_ctx,
                                &r.target_name,
                                &call_arg_ids,
                            )
                        } else {
                            None
                        })
                        .or(res.resolved_yield_type)
                        .or_else(|| match r.kind {
                            EdgeKind::Calls | EdgeKind::Instantiates => {
                                tree.return_type_id_of(res.target_symbol_id)
                            }
                            _ => tree.field_type_id_of(res.target_symbol_id),
                        });
                        // `const { data } = await p.refetch()` — the RHS's own yield
                        // is the async wrapper (`Promise<QueryObserverResult>`), not
                        // what `await` yields; strip one layer before projecting each
                        // destructured field, mirroring the single-identifier peel
                        // above (`unwrap_async_yield_id` gated by `is_awaited`).
                        let recv_ty = if pf.flow.flow_binding_destructure_await.contains(&ref_idx) {
                            recv_ty
                                .map(|id| unwrap_async_yield_id(id, arena, profile.async_wrappers))
                        } else {
                            recv_ty
                        };
                        if let Some(recv_ty) = recv_ty {
                            for (lhs_idx, field_key) in entries {
                                let Some(lhs_sym) = pf.symbols.get(*lhs_idx) else {
                                    continue;
                                };
                                let tuple_index = field_key
                                    .strip_prefix(TUPLE_INDEX_KEY_PREFIX)
                                    .and_then(|index| index.parse::<usize>().ok());
                                let field_ty = if let Some(index) = tuple_index {
                                    match arena.get(recv_ty) {
                                        Type::Tuple(elements) => elements.get(index).copied(),
                                        _ => None,
                                    }
                                } else {
                                    crate::indexer::resolve::engine::chain::field_type_on(
                                        &file_lookup,
                                        arena,
                                        recv_ty,
                                        None,
                                        field_key,
                                    )
                                };
                                if let Some(field_ty) = field_ty {
                                    crate::tracef!(
                                        "SEED destructure lhs='{}' field='{}' -> recorded=TypeId",
                                        lhs_sym.name,
                                        field_key,
                                    );
                                    file_lookup.record_symbol_type(
                                        *lhs_idx,
                                        &lhs_sym.name,
                                        field_ty,
                                    );
                                } else if tuple_index.is_none() {
                                    if let Some(target_id) =
                                        crate::indexer::resolve::engine::chain::callable_member_id_on(
                                        &file_lookup,
                                        arena,
                                        recv_ty,
                                        None,
                                        field_key,
                                    )
                                    {
                                        // The field is a `$Ret` placeholder member with no
                                        // type of its own — record a name-only pointer so a
                                        // later BARE CALL on the binding (`info("hi")`)
                                        // still binds to this exact declaration.
                                        crate::tracef!(
                                            "SEED destructure lhs='{}' field='{}' -> recorded=CallableHead({})",
                                            lhs_sym.name,
                                            field_key,
                                            target_id,
                                        );
                                        file_lookup.record_symbol_callable(
                                            *lhs_idx,
                                            &lhs_sym.name,
                                            target_id,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                edges.push((
                    source_id,
                    callee_id,
                    kind_str,
                    r.line,
                    res.confidence,
                    res.strategy,
                ));
                ref_log.push(RefLog::resolved(source_id, r, &res));
            }
            SolveOutcome::Drained => {
                census.record(ref_idx, Disposition::Drained);
                // A rule positively identified the target as a language builtin
                // or other non-project construct — write the row so it stays
                // diagnosable, but tag it drained so it leaves the rate
                // denominator instead of counting as a genuine miss. A rule
                // decline carries no first-uncaptured-type cause.
                crate::tracef!("RESULT DRAINED (builtin_skip)");
                unresolved.push((
                    source_id,
                    r.target_name.clone(),
                    kind_str,
                    r.line,
                    r.module.clone(),
                    pf.package_id,
                    ref_is_snippet,
                    true,
                    None,
                    None,
                ));
                ref_log.push(RefLog::unresolved(source_id, r, true));
            }
            SolveOutcome::Unresolved(cause) => {
                file_lookup.record_failed_write(ref_idx, cause);
                // A type annotation naming a language primitive (`: string`) is a
                // builtin, not a missing symbol: it's captured as the binding's
                // field type (the compilation pass reads the TypeRef) but must not
                // count as unresolved. Drop it via the profile's primitive set.
                let is_primitive_type = r.kind == EdgeKind::TypeRef
                    && profile
                        .primitive_mapping
                        .iter()
                        .any(|(name, _)| *name == r.target_name);
                if is_primitive_type {
                    census.record(ref_idx, Disposition::Primitive);
                    crate::tracef!("RESULT PRIMITIVE (builtin, not unresolved)");
                } else {
                    census.record(ref_idx, Disposition::Unresolved);
                    crate::tracef!(
                        "RESULT UNRESOLVED cause={}",
                        cause
                            .map(|c| format!("{}(symbol_id={:?})", c.kind.as_db_str(), c.symbol_id))
                            .unwrap_or_else(|| "none".to_string()),
                    );
                    unresolved.push((
                        source_id,
                        r.target_name.clone(),
                        kind_str,
                        r.line,
                        r.module.clone(),
                        pf.package_id,
                        ref_is_snippet,
                        false,
                        cause.and_then(|c| c.symbol_id),
                        cause.map(|c| c.kind.as_db_str()),
                    ));
                    ref_log.push(RefLog::unresolved(source_id, r, false));
                }
            }
        }

        // Collect trace lines for this ref, if any were captured.
        let trace_lines = trace::take_ref();
        if !trace_lines.is_empty() {
            trace::push_traced(trace::TracedRef {
                file: pf.path.clone(),
                line: r.line,
                target: r.target_name.clone(),
                trace_lines,
            });
        }
    }

    (edges, unresolved, ref_log, census)
}

// ---------------------------------------------------------------------------
// EdgeKind → &'static str
// ---------------------------------------------------------------------------

/// Convert an `EdgeKind` to the snake_case `&'static str` used in the DB.
///
/// `EdgeKind` derives `strum::IntoStaticStr` with `serialize_all = "snake_case"`,
/// so the conversion is a zero-cost static dispatch into the strum vtable.
#[inline]
fn edge_kind_str(kind: EdgeKind) -> &'static str {
    kind.into()
}

// ---------------------------------------------------------------------------
// Smoke test
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) use file_context::_test_build_profiles;

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pipeline_trace_tests.rs"]
mod trace_tests;
