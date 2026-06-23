// =============================================================================
// engine/pipeline.rs — single-pass resolution entry for the new engine
//
// Builds a `Compilation` from parse output, materializes the externals the
// project reaches INTO that same tree, resolves every internal ref exactly once
// through `SemanticModel`, and bulk-writes edges + unresolved_refs to the DB. One
// pass, no fixpoint, no old-engine resolution code — externals are resolved like
// internals, the only difference being they are pulled from disk on first sight.
// =============================================================================

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

/// A resolved edge row: (source_id, target_id, kind, source_line, confidence, strategy).
type Edge = (i64, i64, &'static str, u32, f64, &'static str);
/// An unresolved-ref row: (source_id, target_name, kind, source_line, module, package_id, from_snippet).
type Unresolved = (i64, String, &'static str, u32, Option<String>, Option<i64>, bool);

use crate::db::Database;
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::contract::{FileContext, ImportEntry, RefContext, Symbol, SymbolLookup, SymbolSet};
use crate::indexer::resolve::engine::contract::chain_walker::parse_type_head_and_args;
use crate::indexer::resolve::engine::{semantic_model::SemanticModel, compilation::Compilation};
use crate::indexer::resolve::engine::trace;
use crate::indexer::resolve::ResolutionStats;
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::type_checker::profile::language_profile::{ImportModulePath, LanguageProfile};
use crate::types::{AliasTarget, EdgeKind, ParsedFile};
use crate::walker::WalkedFile;

use crate::indexer::resolve::engine::contract::build_scope_chain;

// ---------------------------------------------------------------------------
// Async-wrapper unwrap helper
// ---------------------------------------------------------------------------

/// Peel one async-wrapper layer from `yield_id` when the binding was `await`-ed.
///
/// Handles two forms:
///   - `Type::Apply { base: <wrapper-class>, args: [T] }` where the wrapper
///     class name is in `profile.async_wrappers` — returns `args[0]` (T).
///   - Bare wrapper head with no applied arg — returns `yield_id` unchanged.
///
/// The unwrap fires ONLY at the binding seed (the binding's await flag gates it);
/// a non-awaited `Promise<T>` variable is never touched.
fn unwrap_async_yield_id(
    yield_id: TypeId,
    arena: &TypeArena,
    async_wrappers: &[&str],
) -> TypeId {
    if async_wrappers.is_empty() {
        return yield_id;
    }
    if let Type::Apply { base, args } = arena.get(yield_id) {
        if !args.is_empty() {
            if let Type::Class(head) = arena.get(base) {
                if async_wrappers.contains(&head.as_str()) {
                    return args[0];
                }
            }
        }
    }
    yield_id
}

/// Peel one async-wrapper layer from a type string (the String seed path).
///
/// `"Promise<Response>"` → `"Response"` when `"Promise"` is in `async_wrappers`.
/// Returns `None` when the string has no wrapper head or the first arg is empty,
/// so the caller keeps the original string unchanged.
fn unwrap_async_yield_str<'a>(ty: &'a str, async_wrappers: &[&str]) -> Option<&'a str> {
    if async_wrappers.is_empty() {
        return None;
    }
    let (head, args) = parse_type_head_and_args(ty);
    if args.is_empty() {
        return None;
    }
    if async_wrappers.contains(&head) {
        let inner = args[0].trim();
        if !inner.is_empty() {
            return Some(inner);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// FileLookup — per-file SymbolLookup overlay with a forward-inference cache
// ---------------------------------------------------------------------------

/// A `SymbolLookup` that delegates every structural query to an underlying
/// `Compilation` and overlays a per-file forward-inference cache for local
/// variable types. One instance is constructed per internal file; the cache
/// is populated as the ref loop progresses so a later ref can read the type
/// inferred from an earlier binding (`const x = makeRepo(); x.find()`).
///
/// The cache is intentionally flat (no CFG, no narrowing) — this is forward
/// inference only: LHS-name → yield-type. Two parallel caches are maintained:
/// `locals_id` stores the canonical TypeId directly (populated from
/// `resolved_yield_type` when present, avoiding the `format_type` →
/// `intern_type_str` round-trip that nominalizes primitives/optionals/generics
/// to `Class`); `locals` stores the String fallback for the call sites that
/// still operate on type strings (return_type_str / field_type_str paths).
struct FileLookup<'a> {
    tree: &'a Compilation,
    locals: RefCell<FxHashMap<String, String>>,
    locals_id: RefCell<FxHashMap<String, TypeId>>,
}

impl<'a> FileLookup<'a> {
    fn new(tree: &'a Compilation) -> Self {
        Self {
            tree,
            locals: RefCell::new(FxHashMap::default()),
            locals_id: RefCell::new(FxHashMap::default()),
        }
    }
}

impl<'a> SymbolLookup for FileLookup<'a> {
    // -- Structural delegation: 22 methods forwarded directly to the tree. ----

    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        self.tree.by_name(name)
    }

    fn by_qualified_name(&self, qname: &str) -> Option<&Symbol> {
        self.tree.by_qualified_name(qname)
    }

    fn all_by_qualified_name(&self, qname: &str) -> SymbolSet<'_> {
        self.tree.all_by_qualified_name(qname)
    }

    fn members_of(&self, parent_qname: &str) -> SymbolSet<'_> {
        self.tree.members_of(parent_qname)
    }

    fn members_of_id(&self, parent_id: i64) -> SymbolSet<'_> {
        self.tree.members_of_id(parent_id)
    }

    fn types_by_name(&self, name: &str) -> SymbolSet<'_> {
        self.tree.types_by_name(name)
    }

    fn in_namespace(&self, namespace: &str) -> Vec<&Symbol> {
        self.tree.in_namespace(namespace)
    }

    fn has_in_namespace(&self, namespace: &str) -> bool {
        self.tree.has_in_namespace(namespace)
    }

    fn in_file(&self, file_path: &str) -> SymbolSet<'_> {
        self.tree.in_file(file_path)
    }

    fn ambient_symbols(&self, name: &str) -> SymbolSet<'_> {
        self.tree.ambient_symbols(name)
    }

    fn field_type_name(&self, property_qname: &str) -> Option<&str> {
        self.tree.field_type_name(property_qname)
    }

    fn return_type_name(&self, method_qname: &str) -> Option<&str> {
        self.tree.return_type_name(method_qname)
    }

    fn generic_params(&self, type_name: &str) -> Option<&[String]> {
        self.tree.generic_params(type_name)
    }

    fn field_type_id(&self, property_qname: &str) -> Option<TypeId> {
        self.tree.field_type_id(property_qname)
    }

    fn return_type_id(&self, method_qname: &str) -> Option<TypeId> {
        self.tree.return_type_id(method_qname)
    }

    fn return_type_id_of(&self, symbol_id: i64) -> Option<TypeId> {
        self.tree.return_type_id_of(symbol_id)
    }

    fn type_arena(&self) -> Option<&TypeArena> {
        self.tree.type_arena()
    }

    fn alias_target(&self, name: &str) -> Option<&AliasTarget> {
        self.tree.alias_target(name)
    }

    fn reexports_from(&self, file_path: &str) -> &[(String, String)] {
        self.tree.reexports_from(file_path)
    }

    fn is_external_name(&self, name: &str, language: &str) -> bool {
        self.tree.is_external_name(name, language)
    }

    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        self.tree.parent_class_qname(class_qname)
    }

    fn parent_class_id(&self, child_id: i64) -> Option<i64> {
        self.tree.parent_class_id(child_id)
    }

    fn parent_class_ids(&self, child_id: i64) -> Vec<i64> {
        self.tree.parent_class_ids(child_id)
    }

    fn enclosing_type_qname(&self, source_qname: &str) -> Option<&str> {
        self.tree.enclosing_type_qname(source_qname)
    }

    fn enclosing_namespace_qname(&self, source_qname: &str) -> Option<&str> {
        self.tree.enclosing_namespace_qname(source_qname)
    }

    fn symbols_in_package(&self, package_id: i64) -> SymbolSet<'_> {
        self.tree.symbols_in_package(package_id)
    }

    fn workspace_package_id(&self, specifier: &str) -> Option<i64> {
        self.tree.workspace_package_id(specifier)
    }

    fn is_workspace_declared_name(&self, name: &str) -> bool {
        self.tree.is_workspace_declared_name(name)
    }

    // -- Flow cache: methods implemented over `locals` and `locals_id`. ------

    /// Return the inferred type of `name` from the per-file forward-inference
    /// cache. Returns `None` when the name has not been bound by an earlier ref.
    fn local_type(&self, name: &str) -> Option<String> {
        self.locals.borrow().get(name).cloned()
    }

    /// Single-branch wrapper over `local_type` for the union-aware chain walker.
    fn local_type_union(&self, name: &str) -> Option<Vec<String>> {
        self.local_type(name).map(|t| vec![t])
    }

    /// Bind `name` to `type_name` in the String forward-inference cache. Evicts
    /// any prior TypeId binding for `name` so the two caches never both hold a
    /// stale entry for the same name — a reassignment's latest write wins
    /// regardless of which cache it lands in (`resolve_root` probes `locals_id`
    /// before `locals`).
    fn record_local_type(&self, name: String, type_name: String) {
        self.locals_id.borrow_mut().remove(&name);
        self.locals.borrow_mut().insert(name, type_name);
    }

    /// Return the canonical TypeId binding for `name`. Preferred by the chain
    /// walker root step over `local_type` so non-nominal types (primitives,
    /// optionals, generics) are not nominalized on the round-trip.
    fn local_type_id(&self, name: &str) -> Option<TypeId> {
        self.locals_id.borrow().get(name).copied()
    }

    /// Bind `name` directly to a TypeId, bypassing `format_type` serialization.
    /// Evicts any prior String binding for `name` so a later reassignment that
    /// resolves to a TypeId supersedes an earlier String binding (and vice
    /// versa via `record_local_type`).
    fn record_local_type_id(&self, name: String, id: TypeId) {
        self.locals.borrow_mut().remove(&name);
        self.locals_id.borrow_mut().insert(name, id);
    }

    /// No-op: cursor-based narrowing is deferred; flat forward inference only.
    fn set_cursor(&self, _byte: u32) {}

    /// No-op: narrowing cache installation is deferred; flat forward inference only.
    fn install_local_cache(
        &self,
        _narrowings: Vec<crate::types::Narrowing>,
        _discriminants: Vec<crate::types::DiscriminantNarrowing>,
        _cfg: crate::indexer::flow_cfg::FileCfg,
    ) {
    }

    /// Evict all cached bindings so they cannot bleed into the next file's pass.
    fn clear_local_cache(&self) {
        self.locals.borrow_mut().clear();
        self.locals_id.borrow_mut().clear();
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Single-pass resolution for the rule-based engine.
///
/// Builds a `Compilation` from `parsed`, resolves every ref in every internal
/// file once through `SemanticModel`, then bulk-writes the resulting edges and
/// unresolved_refs to the DB (replacing whatever was there before).
///
/// `project_ctx` supplies the generic project data the engine resolves against
/// (workspace-package names); the `Compilation` snapshots only those generic
/// fields, never language- or ecosystem-specific state.
pub fn resolve_single_pass(
    db: &mut Database,
    parsed: &[ParsedFile],
    symbol_id_map: &HashMap<(String, String), i64>,
    project_ctx: Option<&ProjectContext>,
    arena: Arc<TypeArena>,
    loc: Arc<SymbolLocationIndex>,
) -> Result<ResolutionStats> {
    // Classify ambient globals in the eager batch (internal files + the
    // eagerly-walked externals, including the synthetic TS lib module) so the
    // ambient-scope rung covers lib.*.d.ts / `@types` globals. Classification is
    // ecosystem knowledge; the engine only consults the resulting qname set.
    let ambient_qnames = crate::ecosystem::ambient::ambient_global_qnames(parsed);
    let mut tree = Compilation::build_with_context(
        parsed,
        symbol_id_map,
        Arc::clone(&arena),
        project_ctx,
        &ambient_qnames,
    );

    // Grow the tree with the externals the project reaches. After this the tree
    // holds internal + external symbols and the resolve loop treats them alike.
    materialize_externals(db, &mut tree, parsed, &loc, &arena)
        .context("Failed to materialize external symbols")?;

    // Resolve `ReturnType<typeof fn>` declared return types now that the wrapped
    // (possibly external) functions are materialized, so a wrapper's return type
    // is concrete before forward inference reads it.
    tree.resolve_wrapper_return_types(parsed);
    // Infer a wrapper hook's return from `return <call>` — `function usePost() {
    // return useQuery(...) }` makes usePost's return useQuery's, so a
    // `const { data } = usePost()` destructure roots on the result type. Runs
    // after externals materialize so a wrapper of an external call resolves too.
    tree.infer_call_wrapper_returns(parsed);
    // Type class fields from their call/new initializer — `m = injectMutation(...)`,
    // `#http = inject(HttpClient)` — so `this.m.mutate()` / `this.#http.get()` root.
    tree.infer_field_init_types(parsed);

    let profiles = build_profiles();
    let solver = SemanticModel::production();

    // Resolve every internal file in parallel. Files are independent units of
    // work; refs WITHIN a file stay ordered so forward flow inference (a local's
    // type recorded by an earlier ref is visible to a later ref) is
    // deterministic. The tree is read-only here, shared across workers by ref.
    let per_file: Vec<(Vec<Edge>, Vec<Unresolved>)> = parsed
        .par_iter()
        .filter(|pf| !pf.path.starts_with("ext:"))
        .map(|pf| resolve_one_file(pf, &tree, &profiles, &solver, symbol_id_map))
        .collect();

    let mut edges: Vec<Edge> = Vec::new();
    let mut unresolved: Vec<Unresolved> = Vec::new();
    for (e, u) in per_file {
        edges.extend(e);
        unresolved.extend(u);
    }

    let mut stats = ResolutionStats::default();
    stats.resolved = edges.len() as u64;
    stats.unresolved = unresolved.len() as u64;

    // Write: replace all three resolution tables atomically.
    flush_to_db(db, &edges, &unresolved, true)?;
    // Persist the resolved type metadata so an incremental pass can load it back
    // exactly, instead of re-deriving a lossier version from signatures alone.
    tree.persist_type_info(db.conn())
        .context("Failed to persist symbol type info")?;

    Ok(stats)
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
    symbol_id_map: &HashMap<(String, String), i64>,
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
    for (k, v) in tree.ingest_from_db(db.conn()) {
        full_id_map.entry(k).or_insert(v);
    }

    // Wrapped functions are now loaded from the DB; resolve `ReturnType<typeof fn>`
    // return types before forward inference reads them.
    tree.resolve_wrapper_return_types(parsed);
    // Infer a wrapper hook's return from `return <call>` — `function usePost() {
    // return useQuery(...) }` makes usePost's return useQuery's, so a
    // `const { data } = usePost()` destructure roots on the result type. Runs
    // after externals materialize so a wrapper of an external call resolves too.
    tree.infer_call_wrapper_returns(parsed);
    // Type class fields from their call/new initializer — `m = injectMutation(...)`,
    // `#http = inject(HttpClient)` — so `this.m.mutate()` / `this.#http.get()` root.
    tree.infer_field_init_types(parsed);

    let profiles = build_profiles();
    let solver = SemanticModel::production();

    let per_file: Vec<(Vec<Edge>, Vec<Unresolved>)> = parsed
        .par_iter()
        .filter(|pf| !pf.path.starts_with("ext:"))
        .map(|pf| resolve_one_file(pf, &tree, &profiles, &solver, &full_id_map))
        .collect();

    let mut edges: Vec<Edge> = Vec::new();
    let mut unresolved: Vec<Unresolved> = Vec::new();
    for (e, u) in per_file {
        edges.extend(e);
        unresolved.extend(u);
    }

    let mut stats = ResolutionStats::default();
    stats.resolved = edges.len() as u64;
    stats.unresolved = unresolved.len() as u64;

    // Insert-only: do NOT clear the tables — only the changed files' rows were
    // dropped upstream; everything else must survive.
    flush_to_db(db, &edges, &unresolved, false)?;

    Ok(stats)
}

/// Resolve one internal file's refs, returning its edges and unresolved rows.
/// Refs are visited in source order so a later ref sees the flow-inferred type
/// of a local bound by an earlier ref. No shared mutable state — files run
/// concurrently over the read-only `tree`.
fn resolve_one_file(
    pf: &ParsedFile,
    tree: &Compilation,
    profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
    solver: &SemanticModel,
    symbol_id_map: &HashMap<(String, String), i64>,
) -> (Vec<Edge>, Vec<Unresolved>) {
    let mut edges: Vec<Edge> = Vec::new();
    let mut unresolved: Vec<Unresolved> = Vec::new();

    let Some(&profile) = profiles.get(pf.language.as_str()) else {
        return (edges, unresolved);
    };

    let file_ctx = build_file_context(&pf.language, pf, profile);

    // Fresh per-file flow cache: local bindings from earlier refs in this file
    // are visible to later refs in the same file only.
    let file_lookup = FileLookup::new(tree);

    // Seed locals whose type is declared at the binding site — an explicit
    // annotation (`const x: Array<T> = …`) or a bare literal initializer
    // (`const x = []`). Neither form produces a resolvable RHS ref, so the
    // ref-driven forward inference below would never type them. Seeded before
    // the ref loop so a ref-driven binding for the same name encountered later
    // in the loop still wins (record_local_type overwrites the seeded entry).
    for (&lhs_idx, decl_ty) in &pf.flow.flow_binding_decl_type {
        if let Some(sym) = pf.symbols.get(lhs_idx) {
            file_lookup.record_local_type(sym.name.clone(), decl_ty.clone());
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

    for (ref_idx, r) in pf.refs.iter().enumerate() {
        let Some(source_sym) = pf.symbols.get(r.source_symbol_index) else {
            continue;
        };
        let Some(&source_id) =
            symbol_id_map.get(&(pf.path.clone(), source_sym.qualified_name.clone()))
        else {
            continue;
        };

        // Synthetic primitive-type marker emitted by the extractor — not a
        // resolvable symbol, so it is neither an edge nor an unresolved ref.
        if r.target_name == "_primitive" {
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
        };

        let kind_str = edge_kind_str(r.kind);

        // Activate per-ref tracing when any filter matches this file + line + target.
        // Ref lines are 0-based tree-sitter rows; accept the editor's 1-based line
        // too so either convention matches.
        if trace_filters.iter().any(|(suffix, filter_line, filter_target)| {
            pf.path.ends_with(suffix.as_str())
                && (r.line == *filter_line || r.line + 1 == *filter_line)
                && (filter_target.is_empty() || r.target_name == *filter_target)
        }) {
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
            Some(res) => {
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
                            file_lookup.record_local_type_id(lhs_sym.name.clone(), final_id);
                        } else {
                            // No TypeId from the resolver; derive a String binding
                            // from the target symbol's stored type metadata.
                            let yield_ty = {
                                let target_id = res.target_symbol_id;
                                tree.by_name(&r.target_name)
                                    .iter()
                                    .find(|s| s.id == target_id)
                                    .and_then(|s| match r.kind {
                                        // `const x = f()` — x is f's return type.
                                        EdgeKind::Calls => tree.return_type_str(&s.qualified_name),
                                        // `const x = new Foo()` — x IS Foo.
                                        EdgeKind::Instantiates => Some(s.qualified_name.clone()),
                                        _ => tree.field_type_str(&s.qualified_name),
                                    })
                            };
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
                                file_lookup.record_local_type(lhs_sym.name.clone(), ty);
                            }
                        }
                    }
                } else {
                    crate::tracef!("SEED none (ref is not a binding RHS)");
                }

                edges.push((
                    source_id,
                    res.target_symbol_id,
                    kind_str,
                    r.line,
                    res.confidence,
                    res.strategy,
                ));
            }
            None => {
                crate::tracef!("RESULT UNRESOLVED");

                unresolved.push((
                    source_id,
                    r.target_name.clone(),
                    kind_str,
                    r.line,
                    r.module.clone(),
                    pf.package_id,
                    ref_is_snippet,
                ));
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

    (edges, unresolved)
}

// ---------------------------------------------------------------------------
// DB flush — inline because write_buf::{FileWriteBuf, flush_resolve_buf} are
// pub(super) relative to resolve/, which does not include resolve::engine.
// The logic is identical to flush_resolve_buf with persist_speculative=true.
// ---------------------------------------------------------------------------

fn flush_to_db(
    db: &mut Database,
    edges: &[(i64, i64, &'static str, u32, f64, &'static str)],
    unresolved: &[(i64, String, &'static str, u32, Option<String>, Option<i64>, bool)],
    clear_existing: bool,
) -> Result<()> {
    use rusqlite::types::Value;

    let conn = db.conn();
    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin single-pass resolution transaction")?;

    // The full pass replaces all three tables; the incremental pass inserts only
    // (the changed files' old rows were already dropped upstream when their
    // symbols were rewritten, and the rest of the tables must survive).
    if clear_existing {
        tx.execute("DELETE FROM edges", [])
            .context("Failed to clear edges")?;
        tx.execute("DELETE FROM unresolved_refs", [])
            .context("Failed to clear unresolved_refs")?;
        tx.execute("DELETE FROM external_refs", [])
            .context("Failed to clear external_refs")?;
    }

    const EDGE_CHUNK: usize = 256;
    const UNRESOLVED_CHUNK: usize = 256;

    fn placeholders(rows: usize, cols: usize) -> String {
        let mut s = String::with_capacity(rows * (cols * 2 + 4));
        for i in 0..rows {
            if i > 0 {
                s.push(',');
            }
            s.push('(');
            for j in 0..cols {
                if j > 0 {
                    s.push(',');
                }
                s.push('?');
            }
            s.push(')');
        }
        s
    }

    // Edges: (source_id, target_id, kind, source_line, confidence, strategy)
    if !edges.is_empty() {
        let mut start = 0;
        while start < edges.len() {
            let end = (start + EDGE_CHUNK).min(edges.len());
            let rows = end - start;
            let sql = format!(
                "INSERT OR IGNORE INTO edges \
                 (source_id, target_id, kind, source_line, confidence, strategy) \
                 VALUES {}",
                placeholders(rows, 6),
            );
            let mut params: Vec<Value> = Vec::with_capacity(rows * 6);
            for (sid, tid, kind, line, conf, strat) in &edges[start..end] {
                params.push(Value::Integer(*sid));
                params.push(Value::Integer(*tid));
                params.push(Value::Text((*kind).to_string()));
                params.push(Value::Integer(*line as i64));
                params.push(Value::Real(*conf));
                params.push(Value::Text((*strat).to_string()));
            }
            tx.prepare_cached(&sql)
                .context("Failed to prepare edges insert")?
                .execute(rusqlite::params_from_iter(params.iter()))
                .context("Failed to execute edges insert")?;
            start = end;
        }
    }

    // Unresolved refs: (source_id, target_name, kind, source_line, module, package_id, from_snippet)
    if !unresolved.is_empty() {
        let mut start = 0;
        while start < unresolved.len() {
            let end = (start + UNRESOLVED_CHUNK).min(unresolved.len());
            let rows = end - start;
            let sql = format!(
                "INSERT INTO unresolved_refs \
                 (source_id, target_name, kind, source_line, module, package_id, from_snippet) \
                 VALUES {}",
                placeholders(rows, 7),
            );
            let mut params: Vec<Value> = Vec::with_capacity(rows * 7);
            for (sid, name, kind, line, module, pkg, from_snippet) in &unresolved[start..end] {
                params.push(Value::Integer(*sid));
                params.push(Value::Text(name.clone()));
                params.push(Value::Text((*kind).to_string()));
                params.push(Value::Integer(*line as i64));
                params.push(match module {
                    Some(s) => Value::Text(s.clone()),
                    None => Value::Null,
                });
                params.push(match pkg {
                    Some(v) => Value::Integer(*v),
                    None => Value::Null,
                });
                params.push(Value::Integer(if *from_snippet { 1 } else { 0 }));
            }
            tx.prepare_cached(&sql)
                .context("Failed to prepare unresolved_refs insert")?
                .execute(rusqlite::params_from_iter(params.iter()))
                .context("Failed to execute unresolved_refs insert")?;
            start = end;
        }
    }

    tx.commit()
        .context("Failed to commit single-pass resolution transaction")?;
    Ok(())
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
// Profile map
// ---------------------------------------------------------------------------

/// Build a language-id → LanguageProfile map from the default plugin registry.
///
/// Registers each profile under every language id the plugin claims, matching
/// the same multi-id pattern `Engine::build_from_registry` uses.
fn build_profiles() -> FxHashMap<&'static str, &'static LanguageProfile> {
    let mut profiles: FxHashMap<&'static str, &'static LanguageProfile> = FxHashMap::default();
    for plugin in crate::languages::default_registry().all() {
        if let Some(profile) = plugin.profile() {
            for &lang in plugin.language_ids() {
                profiles.insert(lang, profile);
            }
        }
    }
    profiles
}

// ---------------------------------------------------------------------------
// FileContext builder
// ---------------------------------------------------------------------------

/// Build a `FileContext` from profile data alone, without invoking the Engine.
///
/// Replicates the logic from `engine.rs::generic_file_context`:
/// - `FromModuleField` — any ref with a `module` field becomes an import entry.
/// - Other modes — only `EdgeKind::Imports` refs; `module_path` is either empty
///   (`None` mode) or echoes the target name (`EchoTarget` mode).
fn build_file_context(language: &str, file: &ParsedFile, profile: &LanguageProfile) -> FileContext {
    let imports: Vec<ImportEntry> = match profile.import_module_path {
        // Build entries from import-describing refs only: an explicit import
        // binding (`import { X } from 'm'`) or an `Imports`-kind ref (require /
        // side-effect). A bare usage ref now also carries `module` (set from the
        // import that binds its name), so an unfiltered scan would re-derive a
        // duplicate entry per use site; sourcing the import map from binding refs
        // leaves one entry per imported name while the usage ref's module
        // attribution still reaches the rules via `ctx.r().module`.
        ImportModulePath::FromModuleField => file
            .refs
            .iter()
            .filter(|r| r.is_import_binding || r.kind == EdgeKind::Imports)
            .filter_map(|r| {
                let module = r.module.clone()?;
                Some(ImportEntry {
                    imported_name: r.target_name.clone(),
                    module_path: Some(module),
                    alias: None,
                    is_wildcard: r.target_name == "*",
                })
            })
            .collect(),
        mode => file
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .map(|r| ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: match mode {
                    ImportModulePath::None => None,
                    ImportModulePath::EchoTarget => Some(r.target_name.clone()),
                    ImportModulePath::FromModuleField => unreachable!(),
                },
                alias: None,
                is_wildcard: r.target_name == "*",
            })
            .collect(),
    };
    FileContext {
        file_path: file.path.clone(),
        language: language.to_string(),
        imports,
        file_namespace: None,
    }
}

// ---------------------------------------------------------------------------
// External materialization — pull the externals the project reaches into the
// tree, parse them, write their symbols to the DB, and ingest them. Demand is
// driven by the project's own refs: a ref whose import-resolution tagged a
// `module` names an external symbol; the location index says which file defines
// it. Bounded by reachability — only files defining a referenced name are
// pulled, never whole `node_modules`.
// ---------------------------------------------------------------------------

fn materialize_externals(
    db: &mut Database,
    tree: &mut Compilation,
    parsed: &[ParsedFile],
    loc: &SymbolLocationIndex,
    arena: &Arc<TypeArena>,
) -> Result<()> {
    if loc.is_empty() {
        return Ok(());
    }

    // Seed: the external files defining a name an INTERNAL ref reaches.
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut frontier: Vec<PathBuf> = Vec::new();
    for pf in parsed {
        if pf.path.starts_with("ext:") {
            continue;
        }
        collect_external_files(&pf.refs, tree, loc, &mut seen, &mut frontier);
    }
    if frontier.is_empty() {
        return Ok(());
    }

    // Transitively pull the external type-dependency closure. A materialized
    // external file's own re-exports / imports / extends name the packages that
    // DECLARE the members its API exposes — a test runner re-exports its matcher
    // package whose assertion interface extends another package's, a query helper
    // returns a type aliased into a sibling package — so the member-declaring
    // interfaces stay absent until those are pulled too. Iterated to a fixpoint,
    // bounded: the Roslyn model where resolving a referenced library loads the
    // libraries it references in turn. The same `collect_external_files` seed
    // logic runs over each newly-parsed external file's refs.
    const MAX_CLOSURE_DEPTH: usize = 8;
    let mut ext_parsed: Vec<ParsedFile> = Vec::new();
    let mut to_parse = frontier;
    let mut depth = 0;
    while !to_parse.is_empty() && depth < MAX_CLOSURE_DEPTH {
        let batch: Vec<ParsedFile> = to_parse
            .iter()
            .filter_map(|f| parse_external_file(f, arena))
            .collect();
        let mut next: Vec<PathBuf> = Vec::new();
        for pf in &batch {
            collect_external_files(&pf.refs, tree, loc, &mut seen, &mut next);
        }
        ext_parsed.extend(batch);
        to_parse = next;
        depth += 1;
    }
    if ext_parsed.is_empty() {
        return Ok(());
    }

    // Persist the external symbols (origin='external') to get real DB ids, then
    // ingest them into the tree so refs bind to those ids.
    let (_files, ext_id_map) = crate::indexer::write::write_parsed_files_with_origin(
        db,
        &ext_parsed,
        "external",
        Some(arena),
    )
    .context("Failed to write external symbols")?;
    let ambient_qnames = crate::ecosystem::ambient::ambient_global_qnames(&ext_parsed);
    tree.ingest(&ext_parsed, &ext_id_map, &ambient_qnames);
    Ok(())
}

/// Collect the external files that define a name reached by `refs`, into `out`
/// (deduped via `seen`). A module-tagged ref locates the file exporting the name
/// in that module; an untagged ref pulls only when no internal symbol claims the
/// name — an ambient global, or (in type position) any external definition the
/// `locate` seed missed. Shared by the internal seed pass and the transitive
/// closure passes over already-materialized external files, so a re-export chain
/// into a sibling package is followed the same way an internal import is.
fn collect_external_files(
    refs: &[crate::types::ExtractedRef],
    tree: &Compilation,
    loc: &SymbolLocationIndex,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    for r in refs {
        match r.module.as_deref() {
            // Import-resolution tagged this ref with a module — locate the file
            // that exports the name in that module.
            Some(module) => {
                if let Some(file) = loc.locate(module, &r.target_name) {
                    let file = file.to_path_buf();
                    if seen.insert(file.clone()) {
                        out.push(file);
                    }
                }
            }
            // No module tag. Only pull when the name has no internal definition —
            // an internal symbol always wins over an external.
            None if tree.by_name(&r.target_name).is_empty() => {
                // Import-free global registered under the ambient-scope namespace
                // (`declare global`, test-runner global); bounded to global-
                // declaring packages, so a bare `expect()` materializes its
                // `.d.ts` while an ordinary bare call pulls nothing.
                if let Some(file) =
                    crate::ecosystem::ambient::locate_ambient_global(loc, &r.target_name)
                {
                    let file = file.to_path_buf();
                    if seen.insert(file.clone()) {
                        out.push(file);
                    }
                }
                // A type-position ref may still name an external type the `locate`
                // seed missed (e.g. re-exported through a barrel). Bounded to
                // type-position kinds so a bare method call doesn't pull a
                // same-named external function.
                if matches!(
                    r.kind,
                    EdgeKind::Instantiates
                        | EdgeKind::TypeRef
                        | EdgeKind::Inherits
                        | EdgeKind::Implements
                ) {
                    for (_module, file) in loc.find_by_name(&r.target_name) {
                        let file = file.to_path_buf();
                        if seen.insert(file.clone()) {
                            out.push(file);
                        }
                    }
                }
            }
            None => {}
        }
    }
}

/// Parse one external source file into a `ParsedFile`, consulting the persistent
/// parse cache. Binary-format virtual paths (JAR / DLL) are skipped for now.
/// Mirrors the source path of the old materialize-on-miss driver, but the
/// resulting file is ingested into the new tree rather than the old store.
fn parse_external_file(file: &Path, arena: &Arc<TypeArena>) -> Option<ParsedFile> {
    let path_str = file.to_string_lossy();
    if path_str.starts_with("ext:jar:") || path_str.starts_with("ext:dotnet-type:") {
        return None;
    }

    let language = language_from_file_ext(file)?;
    let virtual_path = virtual_path_for_indexed_file(file, language);
    let bytes = std::fs::read(file).ok()?;
    let hash = crate::indexer::external_parse_cache::content_hash(&bytes);
    let size = bytes.len() as u64;

    if let Some(cached) = crate::indexer::external_parse_cache::get(file, &hash, &virtual_path, size)
    {
        return Some(cached);
    }

    let walked = WalkedFile {
        relative_path: virtual_path,
        absolute_path: file.to_path_buf(),
        language,
    };
    let mut pf = crate::indexer::parse_file::parse_file_with_arena_and_demand(
        &walked,
        crate::languages::default_registry(),
        None,
        arena,
    )
    .ok()?;
    // External `.d.ts` symbols carry a `<pkg>.` prefix the resolver keys on.
    crate::ecosystem::npm::ts_post_process_external(&mut pf);
    crate::indexer::external_parse_cache::put(file, &hash, &pf);
    Some(pf)
}

/// Language id for a pulled file via the registry's extension table.
fn language_from_file_ext(path: &Path) -> Option<&'static str> {
    let name = path.file_name().and_then(|n| n.to_str())?;
    crate::languages::default_registry().language_by_extension(name)
}

/// Virtual path under which a pulled external file is indexed.
fn virtual_path_for_indexed_file(path: &Path, language: &str) -> String {
    crate::indexer::stage_link::virtual_path_for_pulled(path, language)
        .unwrap_or_else(|| format!("ext:idx:{}", path.to_string_lossy().replace('\\', "/")))
}

// ---------------------------------------------------------------------------
// Smoke test
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pipeline_trace_tests.rs"]
mod trace_tests;
