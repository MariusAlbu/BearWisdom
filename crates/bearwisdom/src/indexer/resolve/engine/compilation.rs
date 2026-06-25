// =============================================================================
// engine/compilation.rs — the Compilation: owned symbol store
//
// A lean, owned structural index built directly from parse output — the engine's
// symbol container (Roslyn's `Compilation`). Implements `SymbolLookup` so the
// binder, the rules, and the chain walker all read it through one trait. One
// constructed store per resolution pass; no lifetime threading.
// =============================================================================

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::indexer::resolve::engine::contract::{
    find_matching_bracket, is_jvm_language, merge_where_bounds, parse_generic_param_clause,
    parse_return_type_from_jvm_descriptor, parse_return_type_from_signature,
    parse_return_type_positional, parse_type_head_and_args, resolve_type_name_in_scope,
    FileContext, ImportEntry, Symbol, SymbolLookup, SymbolSet, TypeInfo,
};
use crate::indexer::resolve::engine::support::resolve_module_exported_value_type;
use crate::ecosystem::externals::ts_package_from_virtual_path;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::types::{AliasTarget, EdgeKind, ParsedFile, SymbolKind};

// ---------------------------------------------------------------------------
// `is_type_like` is the single canonical type-name-surface predicate, owned by
// `engine::contract::util`. Aliased here so the call sites read unchanged.
use super::contract::is_type_like_kind as is_type_like;

// ---------------------------------------------------------------------------
// PendingModuleValue — a deferred module-tagged value TypeRef
// ---------------------------------------------------------------------------

/// A value symbol whose first `TypeRef` is module-tagged (`typeof
/// import('m')['k']`). Resolution is deferred to a post-pass so every module's
/// local declarations are typed first, then the export `k` of module `m` is
/// followed to its declaring symbol's type.
struct PendingModuleValue {
    /// The value being typed — its resolved `field_type` slot.
    typed_qname: String,
    /// The imported module specifier (the `m` in `import('m')`).
    module: String,
    /// The exported value name indexed (the `k` in `['k']`); the whole module
    /// when the annotation is the bare `typeof import('m')`.
    key: String,
}

// ---------------------------------------------------------------------------
// Compilation — owned store
// ---------------------------------------------------------------------------

/// Owned structural index of a project's symbols — the engine's `Compilation`.
/// Built once from parse output; implements `SymbolLookup` for the binder, the
/// rules, and the chain walker.
pub struct Compilation {
    by_name: FxHashMap<String, Vec<Symbol>>,
    /// First-winner qualified-name index. Stored as a `BTreeMap` so the
    /// `resolve_type_name_in_scope` helper — which requires a
    /// `&BTreeMap<String, Symbol>` — can borrow it directly without a
    /// copy. `all_by_qualified_name` reads `by_qname_all` for
    /// declaration-merged overloads.
    by_qname: BTreeMap<String, Symbol>,
    by_qname_all: FxHashMap<String, Vec<Symbol>>,
    by_file: FxHashMap<String, Vec<Symbol>>,
    members_by_parent: FxHashMap<String, Vec<Symbol>>,
    /// Id spine: symbol id → its record. The id-keyed counterpart to `by_qname`
    /// (which keys on the qname string); lets consumers holding a resolved id
    /// recover the symbol without a qname round-trip.
    by_id: FxHashMap<i64, Symbol>,
    /// Direct children keyed by PARENT symbol id — the id-keyed counterpart to
    /// `members_by_parent`. Stores child ids (resolved through `by_id`), so a
    /// chain walker that has typed a receiver to a symbol id walks its members
    /// by identity rather than re-matching qname strings. Derived at the end of
    /// `ingest` from the fully-built `members_by_parent` + `by_qname`.
    members_by_id: FxHashMap<i64, Vec<i64>>,
    types_by_name: FxHashMap<String, Vec<Symbol>>,
    /// Per-symbol type metadata: field type, return type, generic params.
    type_info: FxHashMap<String, TypeInfo>,
    /// Per-symbol type metadata keyed by SYMBOL ID — the id-keyed counterpart to
    /// `type_info` (which keys on the qualified-name string). A public API copied
    /// across monorepo packages shares one qname, so the qname slot is
    /// first-writer-wins and binds one package's return type for every copy; the
    /// id slot keeps each declaration's own return distinct, so a chain walker
    /// holding the resolved import-scoped callee id reads that callee's return
    /// rather than the colliding qname winner. Same relationship as
    /// `members_by_id` ↔ `members_by_parent`.
    type_info_by_id: FxHashMap<i64, TypeInfo>,
    /// Re-export map: file_path → [(original_name, source_module)].
    /// Populated from `pf.refs` where `is_reexport` is true.
    reexport_map: FxHashMap<String, Vec<(String, String)>>,
    /// Local export-rename map: module head → (exposed name → declaring qname).
    /// A `module head` is the qualified-name prefix the renamed symbols live
    /// under (an external file's symbols are qnamed `<module>.<name>`), which is
    /// the same string a module-tagged TypeRef carries in its `module` field.
    /// Populated from `is_reexport` refs with no `module` — the in-file
    /// `export { local as exposed }` shape — so resolving the type of an
    /// import-type that indexes the exposed name (`typeof import('m')['exposed']`)
    /// can follow the rename to the local declaration's type.
    export_alias_by_module: FxHashMap<String, FxHashMap<String, String>>,
    /// Direct-parent inheritance: child_qname → ALL parent heads (generics
    /// stripped). An interface or class can extend/implement several supertypes
    /// (`interface A extends X, Y, Z`), so each child keeps every direct parent —
    /// the chain walker must reach a member declared on ANY of them.
    inherits: FxHashMap<String, Vec<String>>,
    /// Generic arguments on each `extends`/`implements` edge: child_qname → ALL
    /// `(parent_head, args)` for that child. `class Child extends Base<User>`
    /// records `("Base", ["User"])`. Parallel to `inherits` (which keeps only the
    /// heads); kept separate so the head-keyed supertype climb is untouched. A
    /// member found on a generic supertype binds that supertype's parameters from
    /// these edge arguments — the arguments live on the edge, not on the receiver.
    inherits_args: FxHashMap<String, Vec<(String, Vec<String>)>>,
    /// Direct-parent inheritance keyed by SYMBOL ID: child symbol id → ALL parent
    /// symbol ids — the id-keyed counterpart of `inherits`. Derived at the end of
    /// `ingest` / `ingest_from_db` by resolving each `inherits` child qname to its
    /// id and each parent head to a SPECIFIC parent symbol (a same-package
    /// candidate winning over a same-named type in another package). Lets the
    /// chain walker climb the supertype DAG by identity, so an `extends Base`
    /// where the `Base` qname is duplicated across packages binds inherited
    /// members from the child's actual base, not the first-wins qname collision —
    /// and a multi-supertype interface reaches members on every branch.
    inherits_by_id: FxHashMap<i64, Vec<i64>>,
    /// Nearest enclosing type-kind ancestor: source_qname → enclosing_type_qname.
    enclosing_type: FxHashMap<String, String>,
    /// Nearest enclosing namespace/module ancestor: source_qname → enclosing_ns_qname.
    enclosing_namespace: FxHashMap<String, String>,
    /// Type-alias targets, keyed by both qualified and simple name. Populated
    /// from `ParsedFile::alias_targets`; consulted by `engine::alias::expand`.
    alias_target: FxHashMap<String, AliasTarget>,
    /// Workspace-package declared name → package id — ecosystem-agnostic (npm,
    /// cargo, go workspaces). Snapshot of `ProjectContext::workspace_pkg_by_declared_name`.
    workspace_pkg_by_declared_name: FxHashMap<String, i64>,
    /// Per-package tsconfig/jsconfig path aliases (`@/` → `./`), snapshot of the
    /// npm manifest's `path_aliases`. A package present here is per-package
    /// isolated — its own aliases apply (even when empty), never the global set —
    /// mirroring `ProjectContext::manifests_for`.
    path_aliases_by_pkg: FxHashMap<i64, Vec<(String, String)>>,
    /// Workspace-wide path aliases, for files not under a per-package manifest.
    path_aliases_global: Vec<(String, String)>,
    /// Symbols grouped by their owning workspace package id, for package-scoped
    /// lookups. Built from each symbol's `package_id` during ingest.
    by_package: FxHashMap<i64, Vec<Symbol>>,
    /// Ambient-scope symbols keyed by simple name: globals a package contributes
    /// without an import. Populated from symbols whose qualified name lives under
    /// the conventional ambient-scope namespace — the marker an ecosystem stamps
    /// on a `declare global` name or test-framework global at materialization.
    ambient_scope: FxHashMap<String, Vec<Symbol>>,
    /// Shared workspace type arena — the same one threaded through extractors.
    arena: Arc<TypeArena>,
    /// Sentinel slices for Borrowed returns that must hand back a `&[…]`.
    empty: Vec<Symbol>,
    empty_pairs: Vec<(String, String)>,
}

impl Compilation {
    /// Build the store from parse output.
    ///
    /// `symbol_id_map` maps `(file_path, qualified_name) → db_id`; symbols
    /// absent from the map are skipped (they were never written to the DB).
    /// `arena` must be the same instance used during extraction so extractor-set
    /// TypeIds on `ExtractedSymbol` (`declared_type`, `return_type`) point into
    /// the canonical table the engine and rules consult.
    pub fn build(
        parsed: &[ParsedFile],
        symbol_id_map: &HashMap<(String, String), i64>,
        arena: Arc<TypeArena>,
    ) -> Self {
        Self::build_with_context(parsed, symbol_id_map, arena, None, &HashSet::new())
    }

    /// Build the store, snapshotting the GENERIC project data the engine needs
    /// from `project_ctx` (workspace-package names). Language- and ecosystem-
    /// specific project state (ambient packages, tsconfig types, framework
    /// registries) is deliberately NOT read here — those concerns live in the
    /// profile / hook / ecosystem layer, not in the generic store.
    ///
    /// `ambient_qnames` is the ambient classification the pipeline computed for
    /// `parsed` (lib.*.d.ts / `@types` globals); it flows to `ingest` so the
    /// ambient-scope rung can bind bare references to those globals.
    pub fn build_with_context(
        parsed: &[ParsedFile],
        symbol_id_map: &HashMap<(String, String), i64>,
        arena: Arc<TypeArena>,
        project_ctx: Option<&ProjectContext>,
        ambient_qnames: &HashSet<String>,
    ) -> Self {
        let mut tree = Self::empty(arena);
        if let Some(ctx) = project_ctx {
            tree.snapshot_project_context(ctx);
        }
        tree.ingest(parsed, symbol_id_map, ambient_qnames);
        tree
    }

    /// Copy the generic, ecosystem-agnostic project data the engine resolves
    /// against: the workspace-package declared-name → id map and the tsconfig /
    /// jsconfig path aliases (per-package and workspace-wide).
    fn snapshot_project_context(&mut self, ctx: &ProjectContext) {
        self.workspace_pkg_by_declared_name = ctx
            .workspace_pkg_by_declared_name
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        // One entry per isolated package (so an alias-less package declines rather
        // than borrowing the global set) plus the workspace-wide fallback. The
        // `AliasedImportRule` consults these via `resolve_path_alias`.
        for (&pkg_id, manifests) in &ctx.by_package {
            let aliases = manifests
                .get(&ManifestKind::Npm)
                .map(|m| m.path_aliases.clone())
                .unwrap_or_default();
            self.path_aliases_by_pkg.insert(pkg_id, aliases);
        }
        if let Some(npm) = ctx.manifests.get(&ManifestKind::Npm) {
            self.path_aliases_global = npm.path_aliases.clone();
        }
    }

    /// An empty store sharing `arena`. Symbols are added via `ingest`.
    fn empty(arena: Arc<TypeArena>) -> Self {
        Self {
            by_name: FxHashMap::default(),
            by_qname: BTreeMap::new(),
            by_qname_all: FxHashMap::default(),
            by_file: FxHashMap::default(),
            members_by_parent: FxHashMap::default(),
            by_id: FxHashMap::default(),
            members_by_id: FxHashMap::default(),
            types_by_name: FxHashMap::default(),
            type_info: FxHashMap::default(),
            type_info_by_id: FxHashMap::default(),
            reexport_map: FxHashMap::default(),
            export_alias_by_module: FxHashMap::default(),
            inherits: FxHashMap::default(),
            inherits_args: FxHashMap::default(),
            inherits_by_id: FxHashMap::default(),
            enclosing_type: FxHashMap::default(),
            enclosing_namespace: FxHashMap::default(),
            alias_target: FxHashMap::default(),
            workspace_pkg_by_declared_name: FxHashMap::default(),
            path_aliases_by_pkg: FxHashMap::default(),
            path_aliases_global: Vec::new(),
            by_package: FxHashMap::default(),
            ambient_scope: FxHashMap::default(),
            arena,
            empty: Vec::new(),
            empty_pairs: Vec::new(),
        }
    }

    /// Add a batch of parsed files into the store. Called first for the
    /// project's internal files, then again to grow the tree with
    /// demand-materialized external files — the seed of lazy node growth. The
    /// first-winner `by_qname` insert keeps an earlier definition; the member
    /// cache is cleared so a newly-added member becomes visible to a re-probe.
    ///
    /// Two-phase structure: the per-file passes (1–4) build all structural
    /// indexes first so `by_qname` is complete for the whole batch before the
    /// typeref-derived type_info pass runs. That pass reads `self.by_qname`
    /// via `resolve_type_name_in_scope` and therefore needs the full batch
    /// visible — not just the symbols emitted so far.
    ///
    /// `ambient_qnames` carries the qualified names the materialization layer
    /// (`ecosystem::ambient`) classified as import-free globals for this batch.
    /// A symbol whose qname is in the set is indexed into `ambient_scope` keyed
    /// by its simple name, for the ambient-scope rung. Empty for the
    /// internal-file batch.
    pub fn ingest(
        &mut self,
        parsed: &[ParsedFile],
        symbol_id_map: &HashMap<(String, String), i64>,
        ambient_qnames: &HashSet<String>,
    ) {
        // -----------------------------------------------------------------------
        // Phase A — structural indexes (Passes 1–4) over all files.
        // -----------------------------------------------------------------------
        for pf in parsed {
            let file_arc: Arc<str> = Arc::from(pf.path.as_str());

            // Pass 1 — symbol indexes for this file.
            for (sym_i, sym) in pf.symbols.iter().enumerate() {
                // Doc-fence / doctest symbols (a ```ts fence in a Markdown API
                // reference, a Rust doctest, a Python `>>>` block) are examples,
                // not the real API — never resolution targets, or a code ref to
                // `useQueryClient` binds to the doc fence shadowing the real
                // function. Markdown-NATIVE symbols are not from_snippet, so doc
                // link resolution is unaffected; symbols stay in the DB for search.
                let from_snippet = pf
                    .symbol_from_snippet
                    .get(sym_i)
                    .copied()
                    .unwrap_or(false);
                if from_snippet {
                    continue;
                }
                let Some(&id) =
                    symbol_id_map.get(&(pf.path.clone(), sym.qualified_name.clone()))
                else {
                    continue;
                };

                let info = Symbol {
                    id,
                    name: sym.name.clone(),
                    qualified_name: sym.qualified_name.clone(),
                    kind: sym.kind.as_str().to_string(),
                    visibility: sym
                        .visibility
                        .as_ref()
                        .map(|v| format!("{v:?}").to_lowercase()),
                    file_path: Arc::clone(&file_arc),
                    scope_path: sym.scope_path.clone(),
                    package_id: pf.package_id,
                    signature: sym.signature.clone(),
                };

                self.by_name
                    .entry(sym.name.clone())
                    .or_default()
                    .push(info.clone());

                self.by_qname_all
                    .entry(sym.qualified_name.clone())
                    .or_default()
                    .push(info.clone());

                self.by_qname
                    .entry(sym.qualified_name.clone())
                    .or_insert_with(|| info.clone());

                self.by_file
                    .entry(pf.path.clone())
                    .or_default()
                    .push(info.clone());

                if let Some(pkg) = pf.package_id {
                    self.by_package.entry(pkg).or_default().push(info.clone());
                }

                // Id spine — first-wins to match `by_qname` (a merged/duplicated
                // qname collapses to one record).
                self.by_id.entry(info.id).or_insert_with(|| info.clone());

                // Ambient scope: the materialization layer (ecosystem::ambient)
                // classified which qnames are import-free globals; index those by
                // simple name so the ambient-scope rung can bind a bare reference.
                if ambient_qnames.contains(&info.qualified_name) {
                    self.ambient_scope
                        .entry(info.name.clone())
                        .or_default()
                        .push(info.clone());
                }

                if is_type_like(&info.kind) {
                    self.types_by_name
                        .entry(sym.name.clone())
                        .or_default()
                        .push(info.clone());
                }

                // members_by_parent — resolve key via parent_index; fall back to
                // qname truncation; top-level symbols key on the empty string.
                let parent_sym = sym.parent_index.and_then(|p| pf.symbols.get(p));
                let parent_key: String = match parent_sym {
                    Some(parent) => parent.qualified_name.clone(),
                    None => match sym.qualified_name.rfind('.') {
                        Some(dot) => sym.qualified_name[..dot].to_string(),
                        None => String::new(),
                    },
                };
                self.members_by_parent
                    .entry(parent_key)
                    .or_default()
                    .push(info.clone());

                // Id-keyed member index keyed on the parent's REAL symbol id (via
                // parent_index → symbol_id_map), not the qname. members_by_parent
                // collapses two same-qname parents in different packages into one
                // bucket; the id index must keep their member sets distinct so the
                // chain walker, having typed a receiver to a specific declaration
                // id, sees only that declaration's members. Top-level symbols (no
                // structural parent) have no type receiver and stay qname-only.
                if let Some(parent) = parent_sym {
                    if let Some(&parent_id) =
                        symbol_id_map.get(&(pf.path.clone(), parent.qualified_name.clone()))
                    {
                        self.members_by_id.entry(parent_id).or_default().push(info.id);
                    }
                }

                // Type metadata from extractor-set TypeIds — render into strings
                // for the legacy string-typed SymbolLookup accessors. Ref-derived
                // type_info fills absent slots in Phase B below.
                if sym.declared_type.is_some() || sym.return_type.is_some() {
                    let ti = self
                        .type_info
                        .entry(sym.qualified_name.clone())
                        .or_insert_with(TypeInfo::default);
                    if let Some(type_id) = sym.declared_type {
                        ti.field_type = Some(self.arena.format_type(type_id));
                        ti.field_type_id = Some(type_id);
                    }
                    if let Some(type_id) = sym.return_type {
                        ti.return_type = Some(self.arena.format_type(type_id));
                        ti.return_type_id = Some(type_id);
                    }
                    // Mirror extractor-set types onto the id-keyed slot (info.id),
                    // so an id-driven read sees the same extractor-wins precedence
                    // as the qname slot.
                    let tid = self
                        .type_info_by_id
                        .entry(info.id)
                        .or_insert_with(TypeInfo::default);
                    if let Some(type_id) = sym.declared_type {
                        tid.field_type = Some(self.arena.format_type(type_id));
                        tid.field_type_id = Some(type_id);
                    }
                    if let Some(type_id) = sym.return_type {
                        tid.return_type = Some(self.arena.format_type(type_id));
                        tid.return_type_id = Some(type_id);
                    }
                }
            }

            // Pass 2 — enclosing_type / enclosing_namespace via parent_index.
            for sym in &pf.symbols {
                let qname = &sym.qualified_name;
                let mut cursor = sym.parent_index;
                let mut found_type: Option<String> = None;
                let mut found_ns: Option<String> = None;
                while let Some(idx) = cursor {
                    let Some(ancestor) = pf.symbols.get(idx) else {
                        break;
                    };
                    let ancestor_kind = ancestor.kind.as_str();
                    if found_type.is_none() && is_type_like(ancestor_kind) {
                        found_type = Some(ancestor.qualified_name.clone());
                    }
                    if found_ns.is_none()
                        && matches!(ancestor_kind, "namespace" | "module")
                    {
                        found_ns = Some(ancestor.qualified_name.clone());
                    }
                    if found_type.is_some() && found_ns.is_some() {
                        break;
                    }
                    cursor = ancestor.parent_index;
                }
                if let Some(t) = found_type {
                    self.enclosing_type.insert(qname.clone(), t);
                }
                if let Some(n) = found_ns {
                    self.enclosing_namespace.insert(qname.clone(), n);
                }
            }

            // Pass 3 — inheritance from refs (Inherits / Implements edges).
            for r in &pf.refs {
                if !matches!(r.kind, EdgeKind::Inherits | EdgeKind::Implements) {
                    continue;
                }
                let Some(child_sym) = pf.symbols.get(r.source_symbol_index) else {
                    continue;
                };
                // Split the head from any generic args. The head keys the inherits
                // map (the supertype climb is head/id-based); the args ride on
                // `inherits_args` so a member found on a generic supertype can bind
                // that supertype's parameters from the `extends Base<Arg>` edge.
                let (parent_head_raw, parent_args) =
                    super::contract::chain_walker::parse_type_head_and_args(&r.target_name);
                let parent_head = parent_head_raw.to_string();
                let parents = self.inherits.entry(child_sym.qualified_name.clone()).or_default();
                if !parents.contains(&parent_head) {
                    parents.push(parent_head.clone());
                }
                if !parent_args.is_empty() {
                    let args = parent_args.iter().map(|a| a.trim().to_string()).collect();
                    self.inherits_args
                        .entry(child_sym.qualified_name.clone())
                        .or_default()
                        .push((parent_head, args));
                }
            }

            // Pass 4 — re-export map from refs where `is_reexport` is true.
            // A re-export ref WITH a module is a cross-module hop (`export { X }
            // from 'm'`). A re-export ref with NO module is an in-file rename
            // (`export { local as exposed }`): `target_name` is the local source
            // name, `namespace_segments[0]` the exposed name. The latter feeds
            // `export_alias_by_module` so import-types that index the exposed
            // name can follow the rename to the local declaration's type.
            for r in &pf.refs {
                if !r.is_reexport {
                    continue;
                }
                if let Some(module) = &r.module {
                    self.reexport_map
                        .entry(pf.path.clone())
                        .or_default()
                        .push((r.target_name.clone(), module.clone()));
                    continue;
                }
                let Some(exposed) = r.namespace_segments.first() else {
                    continue;
                };
                let local = &r.target_name;
                // The local declaration is a same-file symbol whose simple name is
                // `local`; its qname head is the module the exposed name belongs to.
                let Some(local_qname) = pf
                    .symbols
                    .iter()
                    .find(|s| &s.name == local)
                    .map(|s| s.qualified_name.clone())
                else {
                    continue;
                };
                let module_head = local_qname
                    .rsplit_once('.')
                    .map(|(head, _)| head.to_string())
                    .unwrap_or_default();
                self.export_alias_by_module
                    .entry(module_head)
                    .or_default()
                    .insert(exposed.clone(), local_qname);
            }

            // Pass 5 — type-alias targets, keyed by both the qualified and the
            // simple name so a chain's type head matches whether or not it
            // carries its scope prefix.
            for (name, target) in &pf.alias_targets {
                self.alias_target.insert(name.clone(), target.clone());
                if let Some((_, simple)) = name.rsplit_once('.') {
                    self.alias_target
                        .entry(simple.to_string())
                        .or_insert_with(|| target.clone());
                }
            }
        }

        // -----------------------------------------------------------------------
        // Phase B — typeref-derived type_info.
        //
        // `by_qname` is now complete for the whole batch, so
        // `resolve_type_name_in_scope` can scope-qualify type names against every
        // symbol introduced in Phase A. Only fills slots that Phase A left absent
        // (extractor-set TypeIds take precedence).
        // -----------------------------------------------------------------------
        let mut pending_module_values: Vec<PendingModuleValue> = Vec::new();
        for pf in parsed {
            self.derive_type_info_from_refs(pf, symbol_id_map, &mut pending_module_values);
        }

        // -----------------------------------------------------------------------
        // Phase B2 — module-tagged value TypeRefs.
        //
        // Runs after Phase B so every module's local declarations are typed.
        // Each deferred `typeof import('m')['k']` value resolves to the type of
        // the value `m` exports as `k` (following local export renames), instead
        // of self-matching `m.k`.
        // -----------------------------------------------------------------------
        self.resolve_pending_module_values(pending_module_values);

        // -----------------------------------------------------------------------
        // Phase C — bare-identifier return-type inference.
        //
        // Runs after Phase B so a typed parameter's `field_type` is already in
        // `type_info`. Harvests a function's return type from a bare-identifier
        // return (`return qc`) that emits no ref, types the returned identifier
        // against the function's parameter of the same name, and folds candidates
        // through the cross-module agreement gate before filling the return slot.
        // -----------------------------------------------------------------------
        self.infer_bare_identifier_returns(parsed);

        // `members_by_id` is built incrementally in Pass 1 from each member's real
        // parent id — same-qname parents stay distinct — so there is no qname-keyed
        // rebuild here. It accumulates across `ingest` calls (internal files, then
        // materialized externals).

        // Id-keyed inherits, derived from the now-complete `inherits` (child qname
        // → parent head string) + `by_qname`. Each edge resolves to specific
        // symbol ids so the chain walker climbs supertypes by identity.
        self.rebuild_inherits_by_id();
    }

    /// Rebuild `inherits_by_id` from the string-keyed `inherits` map and the
    /// fully-built symbol indexes. For each `child_qname → parent_head` edge,
    /// resolve the child to its id via `by_qname` and the parent head to a
    /// SPECIFIC parent symbol id, preferring a candidate in the child's own
    /// workspace package over a same-named type elsewhere. Rebuilt in full (not
    /// incrementally) because `ingest` / `ingest_from_db` each run over the
    /// cumulative symbol set.
    fn rebuild_inherits_by_id(&mut self) {
        let mut inherits_by_id: FxHashMap<i64, Vec<i64>> = FxHashMap::default();
        for (child_qname, parent_heads) in &self.inherits {
            let Some(child) = self.by_qname.get(child_qname) else {
                continue;
            };
            for parent_head in parent_heads {
                if let Some(parent_id) = self.resolve_parent_id(parent_head, child.package_id) {
                    let parents = inherits_by_id.entry(child.id).or_default();
                    if !parents.contains(&parent_id) {
                        parents.push(parent_id);
                    }
                }
            }
        }
        self.inherits_by_id = inherits_by_id;
    }

    /// Merge TypeScript module-augmentation supertypes into the augmented
    /// interface. A `declare module 'vitest' { interface Assertion extends
    /// TestingLibraryMatchers {} }` (jest-dom) declares its `Assertion` under the
    /// augmenting package, disconnected from the `Assertion` the augmented module
    /// exports — which is what `expect()` returns. Resolve the target through the
    /// augmented module's re-export and graft the augmentation's supertypes onto
    /// it, so a member declared on the augmenting interface's base
    /// (`toBeInTheDocument` on `TestingLibraryMatchers`) resolves on the receiver.
    ///
    /// Each tuple is `(augmented_module, interface_name, augmenting_qname)`. The
    /// augmenting interface's own supertypes are read from `self.inherits`.
    pub(crate) fn apply_module_augmentations(&mut self, augs: &[(String, String, String)]) {
        let mut changed = false;
        for (module, iface, aug_qname) in augs {
            let supertypes = match self.inherits.get(aug_qname) {
                Some(v) if !v.is_empty() => v.clone(),
                _ => continue,
            };
            let Some(target) = self.resolve_module_export_interface(module, iface) else {
                continue;
            };
            if &target == aug_qname {
                continue;
            }
            let entry = self.inherits.entry(target).or_default();
            for s in supertypes {
                if !entry.contains(&s) {
                    entry.push(s);
                    changed = true;
                }
            }
        }
        if changed {
            self.rebuild_inherits_by_id();
        }
    }

    /// The qualified name of the interface that `module` exports as `iface` —
    /// resolved through a re-export declared in a file belonging to `module`
    /// (`vitest`'s `export { Assertion } from '@vitest/expect'` → the
    /// `@vitest/expect.Assertion` interface). Falls back to an interface the
    /// module declares directly under its own package. `None` when neither names
    /// an indexed symbol — the augmentation then merges nowhere rather than
    /// guessing a same-named interface in an unrelated package.
    fn resolve_module_export_interface(&self, module: &str, iface: &str) -> Option<String> {
        for (file, entries) in &self.reexport_map {
            if ts_package_from_virtual_path(file) != Some(module) {
                continue;
            }
            for (name, src_module) in entries {
                if name == iface {
                    let target = format!("{src_module}.{iface}");
                    if self.by_qname.contains_key(&target) {
                        return Some(target);
                    }
                }
            }
        }
        let direct = format!("{module}.{iface}");
        if self.by_qname.contains_key(&direct) {
            return Some(direct);
        }
        None
    }

    /// Resolve a parent head (a qualified or simple name from an `extends` /
    /// `implements` ref) to a specific parent symbol id. A type-like candidate in
    /// the child's own workspace package wins first, so a base whose name is
    /// shared across packages binds to the child's package's base rather than
    /// whichever same-named type won the first-wins qname race. With no
    /// same-package signal, an exact qname match is used, then the first type-like
    /// same-name candidate anywhere.
    fn resolve_parent_id(&self, parent_head: &str, child_package: Option<i64>) -> Option<i64> {
        let simple = parent_head.rsplit('.').next().unwrap_or(parent_head);
        let mut fallback: Option<i64> = None;
        for cand in self.by_name(simple).iter() {
            if !is_type_like(&cand.kind) {
                continue;
            }
            if child_package.is_some() && cand.package_id == child_package {
                return Some(cand.id);
            }
            fallback.get_or_insert(cand.id);
        }
        if let Some(parent) = self.by_qname.get(parent_head) {
            return Some(parent.id);
        }
        fallback
    }

    /// Derive per-symbol type metadata from `TypeRef` refs and signature
    /// strings for one parsed file, writing into `self.type_info`. Slots
    /// already populated by extractor-set TypeIds (Phase A) are left
    /// untouched — merge semantics favour the extractor's richer data.
    ///
    /// Mirrors `populate_materialized_type_info` in `engine/index/lazy.rs`,
    /// using `self.by_qname` (a `BTreeMap`) in place of the eager index's
    /// `self.by_qname`.
    fn derive_type_info_from_refs(
        &mut self,
        pf: &ParsedFile,
        symbol_id_map: &HashMap<(String, String), i64>,
        pending: &mut Vec<PendingModuleValue>,
    ) {
        // Collect TypeRef refs (excluding import bindings) per symbol index. The
        // module tag is preserved alongside the name so the value arm can tell a
        // `typeof import('m')['k']` value-export ref (which self-resolves to the
        // symbol) apart from an ordinary imported type ref.
        let mut type_refs_by_sym: Vec<Vec<(&str, Option<&str>)>> =
            vec![Vec::new(); pf.symbols.len()];
        for r in &pf.refs {
            if r.kind != EdgeKind::TypeRef || r.is_import_binding {
                continue;
            }
            // A chain-bearing TypeRef is a `const x = f(...)` initializer signal:
            // x's type is the call's RETURN type, not the bare callee name `f`.
            // `infer_field_init_types` resolves it from the call's Calls ref; if
            // typed here from `target_name` it would mis-root `x` on the callee.
            if r.chain.is_some() {
                continue;
            }
            if r.source_symbol_index < type_refs_by_sym.len() {
                type_refs_by_sym[r.source_symbol_index]
                    .push((r.target_name.as_str(), r.module.as_deref()));
            }
        }

        for (sym_idx, sym) in pf.symbols.iter().enumerate() {
            let type_refs = &type_refs_by_sym[sym_idx];
            match sym.kind {
                SymbolKind::Property
                | SymbolKind::Field
                | SymbolKind::Variable
                | SymbolKind::Parameter => {
                    // A value whose qname is ALSO an indexed type — the builtin
                    // `declare var Array: ArrayConstructor` paired with `interface
                    // Array` shares the single qname `Array` — must not write its
                    // field_type into the qname slot the type owns. That slot is
                    // read to type INSTANCES of the type (which carry `push` /
                    // `includes` / `map`), so the constructor type mis-binds them.
                    let qname_owned_by_type =
                        self.by_name.get(sym.name.as_str()).is_some_and(|cands| {
                            cands.iter().any(|c| {
                                c.qualified_name == sym.qualified_name && is_type_like(&c.kind)
                            })
                        });
                    // Derive the field type from the first TypeRef (or the jvm /
                    // bare-type-param signature fallbacks) independent of the qname
                    // slot's occupancy, so a second declaration sharing the qname
                    // still records ITS own type in the id slot. Each branch interns
                    // the head with its own semantics (generic args / nominal class /
                    // type-param string). A module-tagged self-resolve defers.
                    let mut derived: Option<(String, Vec<String>, TypeId)> = None;
                    if let Some(&(first, module)) = type_refs.first() {
                        let resolved = resolve_type_name_in_scope(
                            first,
                            sym.scope_path.as_deref(),
                            &self.by_qname,
                        );
                        // A module-tagged ref that scope-resolves to the symbol
                        // ITSELF is the `typeof import('m')['k']` value-export shape:
                        // `k` names a value module `m` exports, not a type, so the
                        // bare-name resolution self-matched. Defer to the post-pass,
                        // which follows the export to the declared value's type once
                        // every module is typed.
                        if module.is_some() && resolved == sym.qualified_name {
                            pending.push(PendingModuleValue {
                                typed_qname: sym.qualified_name.clone(),
                                module: module.unwrap_or_default().to_string(),
                                key: first.to_string(),
                            });
                            continue;
                        }
                        // A non-module type-ref that resolves to the property ITSELF
                        // is a `typeof <same-named value>` collision: scope
                        // qualification matched this member, not the referenced value
                        // (`fn: typeof fn` inside an interface whose member is also
                        // `fn`). Re-bind to a non-self callable of that bare name so a
                        // call on the property yields the named function's return type.
                        let resolved = if resolved == sym.qualified_name {
                            self.by_name
                                .get(first)
                                .and_then(|cands| {
                                    cands.iter().find(|c| {
                                        c.qualified_name != sym.qualified_name
                                            && matches!(c.kind.as_str(), "function" | "method")
                                    })
                                })
                                .map(|c| c.qualified_name.clone())
                                .unwrap_or(resolved)
                        } else {
                            resolved
                        };
                        let arg_strs: Vec<String> = if type_refs.len() > 1 {
                            type_refs[1..].iter().map(|(s, _)| s.to_string()).collect()
                        } else {
                            Vec::new()
                        };
                        let fid = intern_head_and_args(&self.arena, &resolved, &arg_strs);
                        derived = Some((resolved, arg_strs, fid));
                    } else if is_jvm_language(&pf.language) {
                        if let Some(decoded) = sym
                            .signature
                            .as_deref()
                            .and_then(parse_return_type_from_jvm_descriptor)
                        {
                            let resolved = resolve_type_name_in_scope(
                                &decoded,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            let fid = self.arena.class(&resolved);
                            derived = Some((resolved, Vec::new(), fid));
                        }
                    } else if pf.language == "typescript" || pf.language == "tsx" {
                        // A bare type-param field (`value: T` on `class Wrapper<T>`)
                        // has no TypeRef (the post-filter drops it to keep T off the
                        // unresolved-ref surface) but records the param name as the
                        // field's signature. Use it so substitute_through can rebind T
                        // to the applied argument. Guard: bare identifier only —
                        // discriminant literals set signature to a quoted string.
                        if let Some(sig) = sym.signature.as_deref() {
                            if is_bare_type_identifier(sig) {
                                let resolved = resolve_type_name_in_scope(
                                    sig,
                                    sym.scope_path.as_deref(),
                                    &self.by_qname,
                                );
                                let fid = self.arena.intern_type_str(&resolved);
                                derived = Some((resolved, Vec::new(), fid));
                            }
                        }
                    }

                    // A value whose qname is ALSO an indexed type — the builtin
                    // `declare var Array: ArrayConstructor` paired with `interface
                    // Array` shares the single qname `Array` — must not write its
                    // field_type into either slot the type owns: `id_map` can't tell
                    // the two same-qname symbols apart, and the qname slot types
                    // INSTANCES of the type (which carry `push` / `includes` / `map`).
                    if !qname_owned_by_type {
                        if let Some((resolved, arg_strs, fid)) = derived {
                            // Qname slot — first-writer-wins.
                            let ti =
                                self.type_info.entry(sym.qualified_name.clone()).or_default();
                            if ti.field_type.is_none() {
                                ti.field_type_id = Some(fid);
                                ti.field_type = Some(resolved.clone());
                                ti.type_args = arg_strs.clone();
                            }
                            // Id slot — keyed by this symbol's id, kept distinct from a
                            // same-qname first-winner so a caller holding the resolved
                            // id reads THIS field's type.
                            if let Some(&id) = symbol_id_map
                                .get(&(pf.path.clone(), sym.qualified_name.clone()))
                            {
                                let tid = self.type_info_by_id.entry(id).or_default();
                                if tid.field_type.is_none() {
                                    tid.field_type_id = Some(fid);
                                    tid.field_type = Some(resolved);
                                    tid.type_args = arg_strs;
                                }
                            }
                        }
                    }

                    // A callable-typed property (`fn: () => Mock`) yields its
                    // function's return type when CALLED — captured here so
                    // `obj.fn().member` chains continue past the call.
                    let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                    if ti.return_type.is_none() {
                        if let Some(rt) = sym
                            .signature
                            .as_deref()
                            .and_then(parse_return_type_from_signature)
                        {
                            let resolved = resolve_type_name_in_scope(
                                &rt,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            ti.return_type_id = Some(self.arena.intern_type_str(&resolved));
                            ti.return_type = Some(resolved);
                        }
                    }
                }
                SymbolKind::TypeAlias => {
                    if let Some(&(first, _)) = type_refs.first() {
                        let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                        if ti.field_type.is_none() {
                            // Scope-qualify the alias's flattened RHS head to the
                            // declaring module so a transparent alias follows it to
                            // the real declaration (`expect-type.PositiveExpectTypeOf`,
                            // not the bare `PositiveExpectTypeOf`) — the same scope
                            // resolution every field/property type takes.
                            let resolved = resolve_type_name_in_scope(
                                first,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            ti.field_type_id = Some(self.arena.intern_type_str(&resolved));
                            ti.field_type = Some(resolved);
                        }
                    }
                }
                SymbolKind::Method | SymbolKind::Function | SymbolKind::Constructor => {
                    // Signature-based return type, with JVM fallback.
                    let sig_rt: Option<String> = sym.signature.as_deref().and_then(|s| {
                        parse_return_type_from_signature(s)
                            .or_else(|| {
                                if sym.kind == SymbolKind::Constructor {
                                    None
                                } else {
                                    parse_return_type_positional(s)
                                }
                            })
                            .or_else(|| {
                                if is_jvm_language(&pf.language) {
                                    parse_return_type_from_jvm_descriptor(s)
                                } else {
                                    None
                                }
                            })
                    });

                    // When the signature return type carries generic args, prefer
                    // the structured head+args form — it lets the chain walker
                    // bind element types without re-splitting the string.
                    let sig_generic: Option<(String, Vec<String>)> =
                        sig_rt.as_deref().and_then(|rt| {
                            let (head, args) = parse_type_head_and_args(rt);
                            let head_is_name = !head.is_empty()
                                && head
                                    .chars()
                                    .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
                            if args.is_empty() || !head_is_name {
                                None
                            } else {
                                Some((
                                    head.to_string(),
                                    args.iter().map(|s| s.to_string()).collect(),
                                ))
                            }
                        });

                    // This symbol's OWN return type, computed independent of the
                    // shared qname slot — so a same-qname overload in another
                    // package (the qname slot's first-winner) does not hide it.
                    let computed: Option<(String, Option<TypeId>)> =
                        if matches!(sig_rt.as_deref(), Some("this") | Some("Self")) {
                            // A fluent self-returning method (`mockImplementation(
                            // fn): this`) yields the receiver. Its `this` return
                            // emits no TypeRef, so the `type_refs.last()` arm below
                            // would otherwise pick the LAST PARAMETER type — capture
                            // `this` verbatim so the chain walker's self-head rebind
                            // returns the receiver instead.
                            let rid = self.arena.intern_type_str("this");
                            Some(("this".to_string(), Some(rid)))
                        } else if let Some((head, args)) = sig_generic {
                            let resolved = resolve_type_name_in_scope(
                                &head,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            let rid = intern_head_and_args(&self.arena, &resolved, &args);
                            Some((resolved, Some(rid)))
                        } else if let Some(&(last, _)) = type_refs.last() {
                            let resolved = resolve_type_name_in_scope(
                                last,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            let rid = self.arena.intern_type_str(&resolved);
                            Some((resolved, Some(rid)))
                        } else if let Some(rt) = &sig_rt {
                            let resolved = resolve_type_name_in_scope(
                                rt,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            let rid = self.arena.intern_type_str(&resolved);
                            Some((resolved, Some(rid)))
                        } else {
                            None
                        };
                    // A function that returns an object literal (no signature
                    // return type) yields its synthesized `{fn}$Ret` object type,
                    // materialized at parse time — so a call resolves the object's
                    // members.
                    let computed = computed.or_else(|| {
                        let synth = format!("{}$Ret", sym.qualified_name);
                        if self.by_qname.contains_key(&synth) {
                            let tid = self.arena.class(&synth);
                            Some((synth, Some(tid)))
                        } else {
                            None
                        }
                    });
                    if let Some((resolved, rid)) = computed {
                        // Qname slot — first-writer-wins (unchanged).
                        let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                        if ti.return_type.is_none() {
                            ti.return_type_id = rid;
                            ti.return_type = Some(resolved.clone());
                        }
                        // Id slot — keyed by this symbol's id; no qname collision.
                        if let Some(&id) =
                            symbol_id_map.get(&(pf.path.clone(), sym.qualified_name.clone()))
                        {
                            let tid = self.type_info_by_id.entry(id).or_default();
                            if tid.return_type.is_none() {
                                tid.return_type_id = rid;
                                tid.return_type = Some(resolved);
                            }
                        }
                    }
                }
                SymbolKind::Class => {
                    // Constructor call yields the class itself.
                    let class_id = self.arena.class(&sym.qualified_name);
                    let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                    if ti.return_type.is_none() {
                        ti.return_type_id = Some(class_id);
                        ti.return_type = Some(sym.qualified_name.clone());
                    }
                    if let Some(&id) =
                        symbol_id_map.get(&(pf.path.clone(), sym.qualified_name.clone()))
                    {
                        let tid = self.type_info_by_id.entry(id).or_default();
                        if tid.return_type.is_none() {
                            tid.return_type_id = Some(class_id);
                            tid.return_type = Some(sym.qualified_name.clone());
                        }
                    }
                }
                _ => {}
            }
        }

        // Generic-param extraction from signatures for types and callables.
        let bracket_pairs: &[(char, char)] = &[('<', '>'), ('[', ']')];
        for sym in &pf.symbols {
            if !matches!(
                sym.kind,
                SymbolKind::Class
                    | SymbolKind::Interface
                    | SymbolKind::Struct
                    | SymbolKind::TypeAlias
                    | SymbolKind::Function
                    | SymbolKind::Method
            ) {
                continue;
            }
            let Some(sig) = &sym.signature else {
                continue;
            };
            for &(open, close) in bracket_pairs {
                if let Some(start) = sig.find(open) {
                    if let Some(relative_end) =
                        find_matching_bracket(&sig[start..], open, close)
                    {
                        let end = start + relative_end;
                        let mut gparams = parse_generic_param_clause(&sig[start + 1..end]);
                        merge_where_bounds(&mut gparams, sig);
                        if !gparams.is_empty() {
                            let mut params = Vec::with_capacity(gparams.len());
                            let mut bounds = Vec::with_capacity(gparams.len());
                            let mut defaults = Vec::with_capacity(gparams.len());
                            for (n, b, d) in gparams {
                                params.push(n);
                                bounds.push(b);
                                defaults.push(d);
                            }
                            // Key on both simple name and qname so callers
                            // using either form get the params.
                            for key in [&sym.name, &sym.qualified_name] {
                                let ti =
                                    self.type_info.entry(key.clone()).or_default();
                                if ti.generic_params.is_empty() {
                                    ti.generic_params = params.clone();
                                    ti.generic_param_bounds = bounds.clone();
                                    ti.generic_param_defaults = defaults.clone();
                                }
                            }
                            // Id slot — immune to the qname collision that collapses a
                            // name like `useQuery` (shared by several packages and doc
                            // fences) to a single first-winner in the qname slot, so
                            // the real source declarations lose their params. The
                            // (path,qname) map resolves an overload set to its
                            // implementation id, co-locating the overloads' params with
                            // the return the resolver reads there.
                            if let Some(&id) =
                                symbol_id_map.get(&(pf.path.clone(), sym.qualified_name.clone()))
                            {
                                let tid = self.type_info_by_id.entry(id).or_default();
                                if tid.generic_params.is_empty() {
                                    tid.generic_params = params.clone();
                                    tid.generic_param_bounds = bounds.clone();
                                    tid.generic_param_defaults = defaults.clone();
                                }
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Resolve deferred module-tagged value TypeRefs. Each entry's `typed_qname`
    /// receives the `field_type` of the value its module exports as `key`,
    /// following any local export rename. Run after the full Phase B loop so the
    /// exported value's own declared type is already populated. A self-referential
    /// or unresolvable export leaves the slot untouched.
    fn resolve_pending_module_values(&mut self, pending: Vec<PendingModuleValue>) {
        for p in pending {
            let Some((field_type, field_type_id)) = resolve_module_exported_value_type(
                &p.module,
                &p.key,
                &p.typed_qname,
                &self.export_alias_by_module,
                &self.by_qname,
                &self.type_info,
            ) else {
                continue;
            };
            let ti = self.type_info.entry(p.typed_qname).or_default();
            if ti.field_type.is_none() {
                ti.field_type_id =
                    field_type_id.or_else(|| Some(self.arena.class(&field_type)));
                ti.field_type = Some(field_type);
            }
        }
    }

    /// Harvest a function's return type from a bare-identifier return site
    /// (`return qc` / `return queryClient`) that emits no ref — a parameter or
    /// local read is not a cross-symbol reference, so the typeref-derived pass
    /// never saw it. For each `(fn_idx, ident)` the extractor recorded in
    /// `pf.flow.flow_return_ident`, type the returned identifier against the
    /// function's PARAMETER of the same name: a typed param is emitted as a child
    /// symbol scoped to the function, so its declared type lives in
    /// `type_info[{fn_qname}.{ident}].field_type` after Phase B. A candidate is
    /// dropped when the function already has a return (extractor TypeId or a
    /// Phase-A/B return slot), when the param has no resolvable type, when the
    /// type is `unknown`, or when it equals one of the function's generic
    /// parameters (an unbound type variable is not an inference).
    ///
    /// Surviving candidates are folded through `agree_inferred_returns`: a qname
    /// inferred only when every candidate for it agrees on a single type. Because
    /// `type_info` is keyed by qname STRING (first-winner), an agreed type is
    /// sound for every owner of the shared slot — so a hook copied across monorepo
    /// packages (identical qname, distinct symbol id, identical return) infers
    /// correctly, while two same-named functions with different returns leave the
    /// qname uninferred (one slot cannot hold two types).
    fn infer_bare_identifier_returns(&mut self, parsed: &[ParsedFile]) {
        // (fn_qname, candidate_return_type, candidate_return_type_id, type_args)
        let mut candidates: Vec<(String, String, Option<TypeId>, Vec<String>)> = Vec::new();

        for pf in parsed {
            for (fn_idx, ident) in &pf.flow.flow_return_ident {
                let Some(fn_sym) = pf.symbols.get(*fn_idx) else {
                    continue;
                };
                // A declared/extractor return or an already-derived return slot
                // wins; inference only fills genuine gaps.
                if fn_sym.return_type.is_some() {
                    continue;
                }
                if self
                    .type_info
                    .get(&fn_sym.qualified_name)
                    .and_then(|ti| ti.return_type.as_deref())
                    .is_some()
                {
                    continue;
                }
                let param_qname = format!("{}.{}", fn_sym.qualified_name, ident);
                let Some(param_ti) = self.type_info.get(&param_qname) else {
                    continue;
                };
                let Some(ty) = param_ti.field_type.clone() else {
                    continue;
                };
                if ty.is_empty() || ty.eq_ignore_ascii_case("unknown") {
                    continue;
                }
                // An unbound type variable of the function is not an inference.
                let is_generic_param = self
                    .type_info
                    .get(&fn_sym.qualified_name)
                    .map(|ti| ti.generic_params.iter().any(|p| p == &ty))
                    .unwrap_or(false);
                if is_generic_param {
                    continue;
                }
                candidates.push((
                    fn_sym.qualified_name.clone(),
                    ty,
                    param_ti.field_type_id,
                    param_ti.type_args.clone(),
                ));
            }
        }

        for (qname, ty, ty_id, type_args) in agree_inferred_returns(candidates) {
            // Re-check the slot: a same-batch candidate ordering, or a slot a
            // concurrent function already filled, must not be overwritten.
            let ti = self.type_info.entry(qname).or_default();
            if ti.return_type.is_some() {
                continue;
            }
            ti.return_type_id =
                Some(ty_id.unwrap_or_else(|| intern_head_and_args(&self.arena, &ty, &type_args)));
            ti.return_type = Some(ty);
        }
    }

    /// Resolve `ReturnType<typeof fn>`-shaped declared return types in their
    /// DECLARING file's import scope, rewriting `type_info` in place.
    ///
    /// A wrapper `function renderWithClient(...): ReturnType<typeof render>` must
    /// resolve `typeof render` against the file that declares the wrapper — that
    /// file imports `render` — not a use site, which may import only the wrapper
    /// and would bind `render` to a same-named value elsewhere. Resolving once
    /// here stores the concrete return type, so forward inference and the chain
    /// walker both read it with no per-use-site handling.
    ///
    /// Two-phase to satisfy the borrow checker: collect `(qname, type, id)`
    /// rewrites while the tree is read-only, then apply them. Order-independent —
    /// the resolution reads the wrapped function's already-populated return type,
    /// which is never itself a `ReturnType<typeof …>` rewrite target.
    ///
    /// Called by the pipeline AFTER external materialization, so a wrapper of an
    /// external function (`ReturnType<typeof render>` where `render` is
    /// `@testing-library/react`'s) sees that function's return type.
    pub(crate) fn resolve_wrapper_return_types(&mut self, parsed: &[ParsedFile]) {
        let mut rewrites: Vec<(String, String, TypeId)> = Vec::new();
        for pf in parsed.iter().filter(|p| !p.path.starts_with("ext:")) {
            // Import bindings (name → module) for this file. `typeof X` resolves
            // `X` to `{module}.X` via these. TS is the only producer of the
            // `ReturnType<typeof …>` shape, and its import bindings carry `module`.
            let imports: Vec<ImportEntry> = pf
                .refs
                .iter()
                .filter(|r| r.is_import_binding)
                .filter_map(|r| {
                    let module = r.module.clone()?;
                    Some(ImportEntry {
                        imported_name: r.target_name.clone(),
                        module_path: Some(module),
                        alias: None,
                        is_wildcard: r.target_name == "*",
                    })
                })
                .collect();
            if imports.is_empty() {
                continue;
            }
            let file_ctx = FileContext {
                file_path: pf.path.clone(),
                language: pf.language.clone(),
                imports,
                file_namespace: None,
            };
            for sym in &pf.symbols {
                // Cheap pre-filter: only a `typeof`-bearing return type can match.
                let Some(rt) = self
                    .type_info
                    .get(&sym.qualified_name)
                    .and_then(|ti| ti.return_type.clone())
                    .filter(|rt| rt.contains("typeof"))
                else {
                    continue;
                };
                let id = self.arena.intern_type_str(&rt);
                if let Some(resolved) =
                    super::chain::resolve_return_type_extraction(id, self, &self.arena, &file_ctx)
                {
                    let s = self.arena.format_type(resolved);
                    rewrites.push((sym.qualified_name.clone(), s, resolved));
                }
            }
        }
        for (qname, s, id) in rewrites {
            if let Some(ti) = self.type_info.get_mut(&qname) {
                ti.return_type = Some(s);
                ti.return_type_id = Some(id);
            }
        }
    }

    /// Infer a function's return type from a `return <call>` whose value IS the
    /// call's return: `function usePost() { return useQuery(...) }` makes
    /// `usePost`'s return `useQuery`'s. `flow.flow_return_lhs` links the returned
    /// call's ref to its enclosing function; this resolves that call's return in
    /// the function's own import scope (so the right package's overload is read)
    /// and fills the function's return slot. The call-return mirror of
    /// `infer_bare_identifier_returns` (a returned IDENTIFIER), folded through the
    /// same cross-owner agreement gate. Only fills a genuine gap, and only for a
    /// DIRECT call return — a chained `return a.b()` is left to the chain walker.
    pub(crate) fn infer_call_wrapper_returns(&mut self, parsed: &[ParsedFile]) {
        let mut candidates: Vec<(String, String, Option<TypeId>, Vec<String>)> = Vec::new();
        for pf in parsed.iter().filter(|p| !p.path.starts_with("ext:")) {
            if pf.flow.flow_return_lhs.is_empty() {
                continue;
            }
            let imports: Vec<ImportEntry> = pf
                .refs
                .iter()
                .filter(|r| r.is_import_binding)
                .filter_map(|r| {
                    let module = r.module.clone()?;
                    Some(ImportEntry {
                        imported_name: r.target_name.clone(),
                        module_path: Some(module),
                        alias: None,
                        is_wildcard: r.target_name == "*",
                    })
                })
                .collect();
            let file_ctx = FileContext {
                file_path: pf.path.clone(),
                language: pf.language.clone(),
                imports,
                file_namespace: None,
            };
            for (ref_idx, fn_idx) in &pf.flow.flow_return_lhs {
                let Some(fn_sym) = pf.symbols.get(*fn_idx) else {
                    continue;
                };
                // A declared/extractor return annotation wins. An already-INFERRED
                // slot does NOT short-circuit: a factory's object-literal-builder
                // return (`{…}$Ret`) is authoritative and overrides a slot
                // mis-inferred from a param (`Record`) / body (`Promise`) — decided
                // in the grouping pass below.
                if fn_sym.return_type.is_some() {
                    continue;
                }
                let Some(call_ref) = pf.refs.get(*ref_idx) else {
                    continue;
                };
                // Direct call only; a chained `return a.b()` (multi-segment) is
                // the chain walker's job, not this single-callee inference.
                if call_ref.chain.as_ref().map(|c| c.segments.len()).unwrap_or(1) > 1 {
                    continue;
                }
                let Some(ret_id) = super::chain::callee_return_type(
                    self,
                    &self.arena,
                    &file_ctx,
                    &call_ref.target_name,
                ) else {
                    continue;
                };
                let s = self.arena.format_type(ret_id);
                if s.is_empty() || s.eq_ignore_ascii_case("unknown") {
                    continue;
                }
                candidates.push((fn_sym.qualified_name.clone(), s, Some(ret_id), Vec::new()));
            }
        }
        // Group candidates by function. Object-literal-builder returns (`{…}$Ret`)
        // are authoritative: a factory returning one or more local builders HAS
        // that (union of) object shape(s) as its return, overriding any stored
        // return mis-inferred from a param/body. A non-builder single return keeps
        // the agreement-gated, slot-respecting behaviour.
        let mut by_fn: FxHashMap<String, Vec<(String, Option<TypeId>, Vec<String>)>> =
            FxHashMap::default();
        for (qname, ty, ty_id, type_args) in candidates {
            by_fn.entry(qname).or_default().push((ty, ty_id, type_args));
        }
        for (qname, variants) in by_fn {
            // Distinct return-type strings, first occurrence kept.
            let mut distinct: Vec<(String, Option<TypeId>, Vec<String>)> = Vec::new();
            for v in variants {
                if !distinct.iter().any(|d| d.0 == v.0) {
                    distinct.push(v);
                }
            }
            let mut ret_branches: Vec<String> = distinct
                .iter()
                .filter(|d| d.0.ends_with("$Ret"))
                .map(|d| d.0.clone())
                .collect();
            if !ret_branches.is_empty() {
                let (ret_str, ret_id) = if ret_branches.len() == 1 {
                    let n = ret_branches.pop().unwrap();
                    let id = self.arena.class(&n);
                    (n, id)
                } else {
                    ret_branches.sort(); // member-on-all-branches is order-free
                    ret_branches.dedup();
                    let union_name = format!("{qname}$Ret");
                    self.alias_target
                        .insert(union_name.clone(), AliasTarget::Union(ret_branches));
                    let id = self.arena.class(&union_name);
                    (union_name, id)
                };
                self.set_return_both(&qname, ret_str, ret_id);
                continue;
            }
            // No object-literal builder branch: keep a single agreed non-synthetic
            // return, not overriding an existing slot (the wrapper-hook case).
            if self
                .type_info
                .get(&qname)
                .and_then(|ti| ti.return_type.as_deref())
                .is_some()
            {
                continue;
            }
            if distinct.len() == 1 {
                let (ty, ty_id, type_args) = distinct.into_iter().next().unwrap();
                let ti = self.type_info.entry(qname).or_default();
                ti.return_type_id = Some(
                    ty_id.unwrap_or_else(|| intern_head_and_args(&self.arena, &ty, &type_args)),
                );
                ti.return_type = Some(ty);
            }
        }
    }

    /// Write a resolved return type to BOTH stores: the qname slot and every
    /// same-qname symbol's id slot, overriding what is there. The resolver reads
    /// the id slot (`return_type_id_of`) BEFORE the qname slot, and persist
    /// COALESCEs the id store over the qname store — so a qname-only write is
    /// shadowed by a stale id-keyed value.
    fn set_return_both(&mut self, qname: &str, ret_str: String, ret_id: TypeId) {
        let ids: Vec<i64> = self
            .by_qname_all
            .get(qname)
            .map(|v| v.iter().map(|s| s.id).collect())
            .unwrap_or_default();
        let ti = self.type_info.entry(qname.to_string()).or_default();
        ti.return_type_id = Some(ret_id);
        ti.return_type = Some(ret_str.clone());
        for id in ids {
            let tid = self.type_info_by_id.entry(id).or_default();
            tid.return_type_id = Some(ret_id);
            tid.return_type = Some(ret_str.clone());
        }
    }

    /// Type a class field OR local variable from its CALL/NEW initializer:
    /// `readonly m = injectMutation(...)` / `#http = inject(HttpClient)` /
    /// `const router = useRouter()` makes the symbol's type the call's return (or
    /// the constructed class), so `this.m.mutate()` / `this.#http.get()` /
    /// `router.push()` roots on it. (`derive_type_info_from_refs` skips the
    /// chain-bearing initializer TypeRef so it doesn't mis-type the symbol to the
    /// callee name; this pass supplies the resolved return type instead.) The field-initializer call is already
    /// emitted as a Calls/Instantiates ref attributed to the field symbol; the
    /// OUTERMOST one (leftmost byte offset) is the initializer (its arguments,
    /// including a callback's inner calls, anchor to the right). Resolved in the
    /// declaring file's import scope so the right package's overload is read.
    /// Only fills a genuine gap and only for a DIRECT call/new initializer — a
    /// chained `field = a.b().c()` is left to the chain walker.
    pub(crate) fn infer_field_init_types(&mut self, parsed: &[ParsedFile]) {
        let mut updates: Vec<(String, String, TypeId)> = Vec::new();
        for pf in parsed.iter().filter(|p| !p.path.starts_with("ext:")) {
            let imports: Vec<ImportEntry> = pf
                .refs
                .iter()
                .filter(|r| r.is_import_binding)
                .filter_map(|r| {
                    let module = r.module.clone()?;
                    Some(ImportEntry {
                        imported_name: r.target_name.clone(),
                        module_path: Some(module),
                        alias: None,
                        is_wildcard: r.target_name == "*",
                    })
                })
                .collect();
            let file_ctx = FileContext {
                file_path: pf.path.clone(),
                language: pf.language.clone(),
                imports,
                file_namespace: None,
            };
            // Per field symbol, the outermost (leftmost) call/new ref = its
            // initializer. A chained initializer's leftmost ref is multi-segment;
            // skip it (the chain walker types those).
            let mut field_init: FxHashMap<usize, &crate::types::ExtractedRef> = FxHashMap::default();
            for r in &pf.refs {
                if !matches!(r.kind, EdgeKind::Calls | EdgeKind::Instantiates) {
                    continue;
                }
                if r.chain.as_ref().map(|c| c.segments.len()).unwrap_or(1) > 1 {
                    continue;
                }
                let Some(sym) = pf.symbols.get(r.source_symbol_index) else {
                    continue;
                };
                if !matches!(
                    sym.kind,
                    SymbolKind::Property | SymbolKind::Field | SymbolKind::Variable
                ) {
                    continue;
                }
                field_init
                    .entry(r.source_symbol_index)
                    .and_modify(|cur| {
                        if r.byte_offset < cur.byte_offset {
                            *cur = r;
                        }
                    })
                    .or_insert(r);
            }
            for (field_idx, r) in field_init {
                let field = &pf.symbols[field_idx];
                if field.declared_type.is_some() {
                    continue;
                }
                if self
                    .type_info
                    .get(&field.qualified_name)
                    .and_then(|ti| ti.field_type.as_deref())
                    .is_some()
                {
                    continue;
                }
                let ty_id = match r.kind {
                    // `new X()` — the field IS X.
                    EdgeKind::Instantiates => Some(self.arena.class(&r.target_name)),
                    // `call(...)` — the field is the callee's return.
                    _ => super::chain::callee_return_type(self, &self.arena, &file_ctx, &r.target_name),
                };
                let Some(id) = ty_id else {
                    continue;
                };
                let s = self.arena.format_type(id);
                if s.is_empty() || s.eq_ignore_ascii_case("unknown") {
                    continue;
                }
                updates.push((field.qualified_name.clone(), s, id));
            }
        }
        for (qname, s, id) in updates {
            let ti = self.type_info.entry(qname).or_default();
            if ti.field_type.is_some() {
                continue;
            }
            ti.field_type = Some(s);
            ti.field_type_id = Some(id);
        }
    }
}

// ---------------------------------------------------------------------------
// SymbolLookup impl
// ---------------------------------------------------------------------------

impl SymbolLookup for Compilation {
    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.by_name
                .get(name)
                .map(|v| v.as_slice())
                .unwrap_or(&self.empty),
        )
    }

    fn by_qualified_name(&self, qname: &str) -> Option<&Symbol> {
        self.by_qname.get(qname)
    }

    fn all_by_qualified_name(&self, qname: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.by_qname_all
                .get(qname)
                .map(|v| v.as_slice())
                .unwrap_or(&self.empty),
        )
    }

    fn members_of(&self, parent_qname: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.members_by_parent
                .get(parent_qname)
                .map(|v| v.as_slice())
                .unwrap_or(&self.empty),
        )
    }

    fn members_of_id(&self, parent_id: i64) -> SymbolSet<'_> {
        match self.members_by_id.get(&parent_id) {
            Some(ids) => {
                SymbolSet::Owned(ids.iter().filter_map(|id| self.by_id.get(id)).collect())
            }
            None => SymbolSet::empty(),
        }
    }

    fn types_by_name(&self, name: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.types_by_name
                .get(name)
                .map(|v| v.as_slice())
                .unwrap_or(&self.empty),
        )
    }

    fn in_namespace(&self, namespace: &str) -> Vec<&Symbol> {
        let prefix = format!("{namespace}.");
        self.by_qname
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| v)
            .collect()
    }

    fn has_in_namespace(&self, namespace: &str) -> bool {
        let prefix = format!("{namespace}.");
        self.by_qname
            .range(prefix.clone()..)
            .next()
            .map(|(k, _)| k.starts_with(&prefix))
            .unwrap_or(false)
    }

    fn in_file(&self, file_path: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.by_file
                .get(file_path)
                .map(|v| v.as_slice())
                .unwrap_or(&self.empty),
        )
    }

    fn field_type_name(&self, property_qname: &str) -> Option<&str> {
        self.type_info
            .get(property_qname)
            .and_then(|ti| ti.field_type.as_deref())
    }

    fn return_type_name(&self, method_qname: &str) -> Option<&str> {
        self.type_info
            .get(method_qname)
            .and_then(|ti| ti.return_type.as_deref())
    }

    fn generic_params(&self, type_name: &str) -> Option<&[String]> {
        self.type_info
            .get(type_name)
            .filter(|ti| !ti.generic_params.is_empty())
            .map(|ti| ti.generic_params.as_slice())
    }

    fn field_type_id(&self, property_qname: &str) -> Option<TypeId> {
        self.type_info
            .get(property_qname)
            .and_then(|ti| ti.field_type_id)
    }

    fn return_type_id(&self, method_qname: &str) -> Option<TypeId> {
        self.type_info
            .get(method_qname)
            .and_then(|ti| ti.return_type_id)
    }

    fn return_type_id_of(&self, symbol_id: i64) -> Option<TypeId> {
        self.type_info_by_id
            .get(&symbol_id)
            .and_then(|ti| ti.return_type_id)
    }

    fn generic_params_of(&self, symbol_id: i64) -> Option<&[String]> {
        self.type_info_by_id
            .get(&symbol_id)
            .filter(|ti| !ti.generic_params.is_empty())
            .map(|ti| ti.generic_params.as_slice())
    }

    fn generic_param_defaults_of(&self, symbol_id: i64) -> Option<&[Option<String>]> {
        self.type_info_by_id
            .get(&symbol_id)
            .filter(|ti| !ti.generic_params.is_empty())
            .map(|ti| ti.generic_param_defaults.as_slice())
    }

    fn field_type_id_of(&self, symbol_id: i64) -> Option<TypeId> {
        self.type_info_by_id
            .get(&symbol_id)
            .and_then(|ti| ti.field_type_id)
    }

    fn symbol_by_id(&self, id: i64) -> Option<&Symbol> {
        self.by_id.get(&id)
    }

    fn type_arena(&self) -> Option<&TypeArena> {
        Some(&self.arena)
    }

    fn reexports_from(&self, file_path: &str) -> &[(String, String)] {
        self.reexport_map
            .get(file_path)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty_pairs)
    }

    fn is_external_name(&self, _name: &str, _language: &str) -> bool {
        // External classification is a later phase; conservative false here.
        false
    }

    fn ambient_symbols(&self, name: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.ambient_scope
                .get(name)
                .map(|v| v.as_slice())
                .unwrap_or(&self.empty),
        )
    }

    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        self.inherits
            .get(class_qname)
            .and_then(|v| v.first())
            .map(|s| s.as_str())
    }

    fn parent_class_qnames(&self, class_qname: &str) -> &[String] {
        self.inherits.get(class_qname).map(|v| v.as_slice()).unwrap_or(&[])
    }

    fn parent_class_id(&self, child_id: i64) -> Option<i64> {
        self.inherits_by_id
            .get(&child_id)
            .and_then(|v| v.first())
            .copied()
    }

    fn parent_class_ids(&self, child_id: i64) -> Vec<i64> {
        self.inherits_by_id.get(&child_id).cloned().unwrap_or_default()
    }

    fn parent_class_args(&self, child_head: &str, parent_head: &str) -> &[String] {
        self.inherits_args
            .get(child_head)
            .and_then(|edges| edges.iter().find(|(h, _)| h == parent_head))
            .map(|(_, args)| args.as_slice())
            .unwrap_or(&[])
    }

    fn enclosing_type_qname(&self, source_qname: &str) -> Option<&str> {
        self.enclosing_type
            .get(source_qname)
            .map(|s| s.as_str())
    }

    fn enclosing_namespace_qname(&self, source_qname: &str) -> Option<&str> {
        self.enclosing_namespace
            .get(source_qname)
            .map(|s| s.as_str())
    }

    fn alias_target(&self, name: &str) -> Option<&AliasTarget> {
        self.alias_target.get(name)
    }

    fn workspace_package_id(&self, specifier: &str) -> Option<i64> {
        if let Some(&id) = self.workspace_pkg_by_declared_name.get(specifier) {
            return Some(id);
        }
        // Deep import: peel trailing `/seg` until the declared name matches.
        let mut path = specifier;
        while let Some(slash) = path.rfind('/') {
            path = &path[..slash];
            if let Some(&id) = self.workspace_pkg_by_declared_name.get(path) {
                return Some(id);
            }
        }
        None
    }

    fn is_workspace_declared_name(&self, name: &str) -> bool {
        self.workspace_pkg_by_declared_name.contains_key(name)
    }

    fn resolve_path_alias(&self, package_id: Option<i64>, specifier: &str) -> Option<String> {
        // Per-package isolation: an isolated package uses its own aliases (even
        // when empty); a file outside the per-package map uses the global set.
        // Mirrors `ProjectContext::resolve_path_alias` — longest alias prefix wins.
        let aliases = match package_id.and_then(|id| self.path_aliases_by_pkg.get(&id)) {
            Some(per_pkg) => per_pkg.as_slice(),
            None => self.path_aliases_global.as_slice(),
        };
        let mut best: Option<&(String, String)> = None;
        for entry in aliases {
            if specifier.starts_with(entry.0.as_str())
                && best.map_or(true, |(b, _)| entry.0.len() > b.len())
            {
                best = Some(entry);
            }
        }
        let (alias, target) = best?;
        Some(format!("{target}{}", &specifier[alias.len()..]))
    }

    fn symbols_in_package(&self, package_id: i64) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.by_package
                .get(&package_id)
                .map(|v| v.as_slice())
                .unwrap_or(&self.empty),
        )
    }
}

// ---------------------------------------------------------------------------
// DB persistence — the incremental complement of build_with_context
// ---------------------------------------------------------------------------

impl Compilation {
    /// Persist per-symbol resolved type metadata to `symbol_type_info`, so an
    /// incremental pass loads back exactly the full pass's ref+signature-derived
    /// type info for unchanged symbols (not the lossy signature-only
    /// re-derivation). Replaces the table wholesale. Only entries that map to a
    /// DB symbol id are written; the simple-name `generic_params` duplicate keys
    /// are rebuilt from these rows on load.
    pub(crate) fn persist_type_info(&self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM symbol_type_info", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO symbol_type_info \
                 (symbol_id, field_type, return_type, type_args, generic_params) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (qname, ti) in &self.type_info {
                let Some(sym) = self.by_qname.get(qname) else {
                    continue;
                };
                if ti.field_type.is_none()
                    && ti.return_type.is_none()
                    && ti.type_args.is_empty()
                    && ti.generic_params.is_empty()
                {
                    continue;
                }
                stmt.execute(rusqlite::params![
                    sym.id,
                    ti.field_type.as_deref(),
                    ti.return_type.as_deref(),
                    json_string_array(&ti.type_args),
                    json_string_array(&ti.generic_params),
                ])?;
            }
        }
        // Per-id rows from `type_info_by_id`. The qname loop above writes one row
        // per qualified name (first-winner), so a same-qname overload set — every
        // `HttpClient.get`, every `inject` — persists only one declaration's
        // return. Upsert each symbol id's OWN field/return here so an incremental
        // reload restores the overload-accurate metadata the resolver reads by id;
        // COALESCE keeps the qname loop's `generic_params` / `type_args` columns.
        {
            let mut stmt = tx.prepare(
                "INSERT INTO symbol_type_info (symbol_id, field_type, return_type, generic_params) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(symbol_id) DO UPDATE SET \
                   field_type = COALESCE(excluded.field_type, field_type), \
                   return_type = COALESCE(excluded.return_type, return_type), \
                   generic_params = COALESCE(excluded.generic_params, generic_params)",
            )?;
            for (id, ti) in &self.type_info_by_id {
                if ti.field_type.is_none() && ti.return_type.is_none() && ti.generic_params.is_empty()
                {
                    continue;
                }
                let generic_params = (!ti.generic_params.is_empty())
                    .then(|| json_string_array(&ti.generic_params));
                stmt.execute(rusqlite::params![
                    id,
                    ti.field_type.as_deref(),
                    ti.return_type.as_deref(),
                    generic_params,
                ])?;
            }
        }
        tx.commit()
    }

    /// Load every DB symbol — and its persisted type metadata — into the store,
    /// skipping qnames already present from the freshly-parsed batch. The
    /// incremental complement of `build_with_context`: changed files come from
    /// `parsed`, the unchanged remainder (plus the externals the last full index
    /// materialized) come from here, so cross-file chains into untouched code
    /// still resolve. No on-disk externals walk runs on a save.
    pub(crate) fn ingest_from_db(
        &mut self,
        conn: &rusqlite::Connection,
    ) -> HashMap<(String, String), i64> {
        // Complete `(path, qname) -> id` map over every DB symbol — the resolve
        // loop maps each source symbol to its id through it, and an affected
        // (re-resolved) file's source symbols are only in the DB, not the passed
        // changed-files map.
        let mut id_map: HashMap<(String, String), i64> = HashMap::new();

        // The freshly-parsed (changed) symbols, captured before DB symbols are
        // folded in. Their type_info is authoritative from this parse, so a stale
        // persisted row must NOT resurrect type info the edit removed.
        let parsed_qnames: HashSet<String> = self.by_qname.keys().cloned().collect();

        // 1) Symbols + structural indexes.
        let Ok(mut stmt) = conn.prepare(
            "SELECT s.id, s.name, s.qualified_name, s.kind, f.path, s.scope_path, \
                    s.visibility, f.package_id, s.signature, s.containing_id \
             FROM symbols s JOIN files f ON f.id = s.file_id",
        ) else {
            return id_map;
        };
        let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<i64>>(9)?,
            ))
        }) else {
            return id_map;
        };
        for (id, name, qname, kind, path, scope_path, visibility, package_id, signature, containing_id) in
            rows.flatten()
        {
            // Every DB symbol contributes to the id map, even ones the parsed
            // batch already owns.
            id_map.insert((path.clone(), qname.clone()), id);

            // The freshly-parsed batch wins for the structural indexes.
            if self.by_qname.contains_key(&qname) {
                continue;
            }
            let file_arc: std::sync::Arc<str> = std::sync::Arc::from(path.as_str());
            let info = Symbol {
                id,
                name: name.clone(),
                qualified_name: qname.clone(),
                kind: kind.clone(),
                visibility,
                file_path: std::sync::Arc::clone(&file_arc),
                scope_path,
                package_id,
                signature,
            };
            self.by_name.entry(name.clone()).or_default().push(info.clone());
            self.by_qname_all.entry(qname.clone()).or_default().push(info.clone());
            self.by_qname.entry(qname.clone()).or_insert_with(|| info.clone());
            self.by_file.entry(path).or_default().push(info.clone());
            self.by_id.entry(id).or_insert_with(|| info.clone());
            if let Some(pkg) = package_id {
                self.by_package.entry(pkg).or_default().push(info.clone());
            }
            if is_type_like(&kind) {
                self.types_by_name.entry(name).or_default().push(info.clone());
            }
            let parent_key = match qname.rfind('.') {
                Some(dot) => qname[..dot].to_string(),
                None => String::new(),
            };
            self.members_by_parent.entry(parent_key).or_default().push(info);

            // Id-keyed membership from the DB's structural-parent edge (the real
            // parent symbol id), keeping same-qname parents distinct — the same
            // identity index Pass 1 builds for freshly-parsed symbols.
            if let Some(parent_id) = containing_id {
                self.members_by_id.entry(parent_id).or_default().push(id);
            }
        }

        // 2) Persisted type_info, re-interned into this build's fresh arena.
        if let Ok(mut ti_stmt) = conn.prepare(
            "SELECT s.qualified_name, s.name, t.field_type, t.return_type, \
                    t.type_args, t.generic_params, t.symbol_id \
             FROM symbol_type_info t JOIN symbols s ON s.id = t.symbol_id",
        ) {
            if let Ok(ti_rows) = ti_stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            }) {
                for (qname, name, field_type, return_type, type_args_j, generic_params_j, symbol_id) in
                    ti_rows.flatten()
                {
                    // A changed symbol's type_info is the fresh parse's, not the
                    // last index's persisted (possibly stale) row.
                    if parsed_qnames.contains(&qname) {
                        continue;
                    }
                    let type_args = parse_json_string_array(type_args_j.as_deref());
                    let generic_params = parse_json_string_array(generic_params_j.as_deref());

                    let ti = self.type_info.entry(qname.clone()).or_default();
                    if ti.field_type.is_none() {
                        if let Some(ft) = &field_type {
                            ti.field_type_id = Some(self.arena.intern_type_str(ft));
                            ti.field_type = Some(ft.clone());
                        }
                    }
                    if ti.return_type.is_none() {
                        if let Some(rt) = &return_type {
                            ti.return_type_id = Some(self.arena.intern_type_str(rt));
                            ti.return_type = Some(rt.clone());
                        }
                    }
                    if ti.type_args.is_empty() {
                        ti.type_args = type_args;
                    }
                    if ti.generic_params.is_empty() && !generic_params.is_empty() {
                        ti.generic_params = generic_params.clone();
                    }
                    // Rebuild the simple-name generic_params key (the full pass
                    // stores generic params under both the qname and the bare name).
                    if !generic_params.is_empty() && name != qname {
                        let sti = self.type_info.entry(name).or_default();
                        if sti.generic_params.is_empty() {
                            sti.generic_params = generic_params.clone();
                        }
                    }
                    // Id-keyed slot — the persisted row is per-symbol-id, so this
                    // restores the overload-accurate return the resolver reads by
                    // id (`return_type_id_of`) on an incremental reload, with no
                    // qname-collapse.
                    let tid = self.type_info_by_id.entry(symbol_id).or_default();
                    if tid.field_type.is_none() {
                        if let Some(ft) = &field_type {
                            tid.field_type_id = Some(self.arena.intern_type_str(ft));
                            tid.field_type = Some(ft.clone());
                        }
                    }
                    if tid.return_type.is_none() {
                        if let Some(rt) = &return_type {
                            tid.return_type_id = Some(self.arena.intern_type_str(rt));
                            tid.return_type = Some(rt.clone());
                        }
                    }
                    if tid.generic_params.is_empty() && !generic_params.is_empty() {
                        tid.generic_params = generic_params;
                    }
                }
            }
        }

        // 3) Inheritance from persisted edges. The changed files' edges were
        //    cleared upstream before this pass, so the rows are the unchanged
        //    remainder; the changed files' inherits come from `parsed`.
        if let Ok(mut inh) = conn.prepare(
            "SELECT src.qualified_name, tgt.qualified_name \
             FROM edges e \
             JOIN symbols src ON src.id = e.source_id \
             JOIN symbols tgt ON tgt.id = e.target_id \
             WHERE e.kind IN ('inherits', 'implements')",
        ) {
            if let Ok(inh_rows) =
                inh.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            {
                for (child, parent) in inh_rows.flatten() {
                    // `edges.target` is the RESOLVED parent symbol's qname (the bare
                    // head); the `extends Base<Arg>` generic args are not on the edge
                    // row, so `inherits_args` cannot be recovered on this incremental
                    // reload path — the supertype-arg binding is populated only by the
                    // full-reindex Pass 3 (which reads `r.target_name` with its args).
                    let head = parent.split('<').next().unwrap_or(&parent).to_string();
                    let parents = self.inherits.entry(child).or_default();
                    if !parents.contains(&head) {
                        parents.push(head);
                    }
                }
            }
        }

        // `members_by_id` was built directly from each DB symbol's `containing_id`
        // (its real parent id) in the loop above, alongside Pass 1's id-keyed
        // membership for the freshly-parsed symbols — same-qname parents stay
        // distinct, so no qname-keyed rebuild here.

        // Id-keyed inherits over the now-complete maps (parsed batch + DB rows).
        self.rebuild_inherits_by_id();

        id_map
    }
}

/// Fold harvested return-type candidates into per-qname agreed returns.
///
/// Each candidate is `(fn_qname, return_type, return_type_id, type_args)`. A
/// qname's return is agreed only when every candidate for it carries the same
/// `return_type` string; any disagreement leaves the qname out. Agreement is the
/// soundness gate for the qname-string-keyed return slot, and it holds across
/// owners: an agreed type is correct for every owner of the shared slot, so a
/// function copied across monorepo packages (identical qname, distinct symbol
/// id, identical return) is folded, while two same-named functions with
/// different returns are skipped — one slot cannot hold two types. The agreed
/// candidate's id and type args ride along so the caller fills the structured
/// slot, not just the string.
fn agree_inferred_returns(
    candidates: Vec<(String, String, Option<TypeId>, Vec<String>)>,
) -> Vec<(String, String, Option<TypeId>, Vec<String>)> {
    // qname → Some(candidate) on a single agreed type, None on a conflict.
    let mut by_fn: HashMap<String, Option<(String, Option<TypeId>, Vec<String>)>> = HashMap::new();
    for (qname, ty, ty_id, type_args) in candidates {
        match by_fn.get(&qname) {
            None => {
                by_fn.insert(qname, Some((ty, ty_id, type_args)));
            }
            Some(Some((prev_ty, _, _))) if *prev_ty != ty => {
                by_fn.insert(qname, None);
            }
            _ => {}
        }
    }
    by_fn
        .into_iter()
        .filter_map(|(qname, agreed)| {
            agreed.map(|(ty, ty_id, type_args)| (qname, ty, ty_id, type_args))
        })
        .collect()
}

/// Serialize a string vec to a JSON array, or `None` (SQL NULL) when empty.
fn json_string_array(items: &[String]) -> Option<String> {
    if items.is_empty() {
        None
    } else {
        serde_json::to_string(items).ok()
    }
}

/// Parse a JSON string array back to a vec; empty on NULL or malformed input.
fn parse_json_string_array(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default()
}

/// Intern a head qname plus its (string) generic arguments into a single
/// canonical TypeId: a bare `Class(head)` when there are no args, else
/// `Apply { Class(head), [args…] }`. The chain walker reads this id directly, so
/// a ref-derived field/return type needs no per-hop string re-parse.
fn intern_head_and_args(arena: &TypeArena, head: &str, args: &[String]) -> TypeId {
    let base = arena.class(head);
    if args.is_empty() {
        base
    } else {
        let arg_ids = args.iter().map(|a| arena.intern_type_str(a)).collect();
        arena.intern(Type::Apply { base, args: arg_ids })
    }
}

/// Returns `true` when `s` is a bare type-identifier: starts with an ASCII
/// letter or underscore, contains only `[A-Za-z0-9_]`, and is not a
/// TypeScript primitive-value literal (`true`, `false`, `null`, `undefined`).
/// Excludes quoted string discriminants, numeric literals, and arrow-type
/// signatures (`() => T`).
fn is_bare_type_identifier(s: &str) -> bool {
    if matches!(s, "true" | "false" | "null" | "undefined") {
        return false;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Test-only re-export of the type-like predicate so `tree_tests.rs` can
/// assert that `types_by_name` only surfaces type-like symbols without
/// duplicating the match arms.
#[cfg(test)]
pub(super) fn is_type_like_for_test(kind: &str) -> bool {
    is_type_like(kind)
}

/// Test-only re-export of the return-inference agreement gate so the sibling
/// tests can drive it on candidate tuples directly — the param-qname collision
/// that masks per-owner type differences in the full pass makes the gate's
/// disagreement branch only reachable by feeding candidates straight in.
#[cfg(test)]
pub(super) fn agree_inferred_returns_for_test(
    candidates: Vec<(String, String, Option<TypeId>, Vec<String>)>,
) -> Vec<(String, String, Option<TypeId>, Vec<String>)> {
    agree_inferred_returns(candidates)
}

#[cfg(test)]
#[path = "compilation_tests.rs"]
mod tests;
