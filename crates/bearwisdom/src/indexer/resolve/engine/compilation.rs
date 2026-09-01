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

use rustc_hash::{FxHashMap, FxHashSet};

use crate::indexer::resolve::engine::ext_lang_visibility::ExtLangVisibility;
use crate::indexer::resolve::engine::import_qualify;
use crate::indexer::resolve::engine::module_specifier;
use crate::indexer::resolve::engine::contract::{
    is_jvm_language, parse_object_type_members, parse_param_types_from_signature,
    parse_return_type_from_jvm_descriptor, parse_return_type_from_signature,
    parse_return_type_positional, parse_top_level_conditional, parse_type_head_and_args,
    build_scope_chain, resolve_type_name_in_scope, signature_generic_params,
    FileContext, ImportEntry, RefContext, Symbol, SymbolLookup, SymbolSet, TypeInfo,
};
use crate::indexer::resolve::engine::support::resolve_module_exported_value_type;
use crate::ecosystem::externals::ts_package_from_virtual_path;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::write::SymbolIds;
use crate::indexer::project_context::ProjectContext;
use crate::type_checker::core::types::{GenericParamData, GenericParamId, Type, TypeArena, TypeId};
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{
    intern_alias_target, AliasTarget, AliasTargetIds, EdgeKind, ExtractedSymbol, ParsedFile,
    SymbolKind,
};

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
    pub(super) by_qname: BTreeMap<String, Symbol>,
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
    pub(super) type_info: FxHashMap<String, TypeInfo>,
    /// Per-symbol type metadata keyed by SYMBOL ID — the id-keyed counterpart to
    /// `type_info` (which keys on the qualified-name string). A public API copied
    /// across monorepo packages shares one qname, so the qname slot is
    /// first-writer-wins and binds one package's return type for every copy; the
    /// id slot keeps each declaration's own return distinct, so a chain walker
    /// holding the resolved import-scoped callee id reads that callee's return
    /// rather than the colliding qname winner. Same relationship as
    /// `members_by_id` ↔ `members_by_parent`.
    pub(super) type_info_by_id: FxHashMap<i64, TypeInfo>,
    /// Deferred import-scoped head requalification, retained across `ingest`
    /// calls so an import whose package materializes in a LATER batch still
    /// rewrites the earlier batch's annotation heads. See `import_qualify`.
    pending_import_requalify: Vec<import_qualify::PendingFile>,
    /// Re-export map: file_path → [(original_name, source_module)].
    /// Populated from `pf.refs` where `is_reexport` is true.
    reexport_map: FxHashMap<String, Vec<(String, String)>>,
    /// Bare module specifier → the indexed external file that re-export following
    /// starts from. Keys a package's import specifier (`vue`, `@vue/runtime-dom`)
    /// to its entry/barrel `ext:<lang>:<pkg>/…` file, so a bare named import drives
    /// `follow_reexports` from the package entry. Built from the shared
    /// `ext:<lang>:<pkg>` path convention; no per-language code.
    module_entry: FxHashMap<String, String>,
    /// `module_specifier`'s `ModuleResolver`-fallback state.
    module_specifier: module_specifier::Context,
    /// Angular/CSS selector → class qualified-name, from `ParsedFile::component_selectors`.
    /// Backs `SymbolLookup::selector_qname`, which `SelectorMapRule` consults to bind a
    /// selector ref (`<nb-card>` → `NbCardComponent`). First-writer-wins.
    selector_to_qname: FxHashMap<String, String>,
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
    /// Interned-id form of `inherits_args` (arg type-heads as `TypeId`) so the
    /// supertype-arg substitution binds without re-`intern_type_str`ing the
    /// stored arg strings on each access. Populated on full build; empty after an
    /// incremental DB reload (the edge args aren't recoverable there).
    inherits_arg_ids: FxHashMap<String, Vec<(String, Vec<TypeId>)>>,
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
    /// Generic args of an `extends`/`implements` edge, keyed by the RESOLVED
    /// (child_id, parent_id) pair — the identity twin of `inherits_arg_ids`.
    inherits_args_by_pair: FxHashMap<(i64, i64), Vec<TypeId>>,
    /// Declaration-merging buckets recorded in Pass 1 (profile `MergeScope`).
    merge_groups: super::merge_canonical::MergeGroups,
    /// Non-canonical merge-set row id → the set's canonical (smallest) id.
    merge_canonical: FxHashMap<i64, i64>,
    /// `(child_qname, parent_head) → import module`: the child file's import
    /// of that exact head, captured when the edge was recorded. The one
    /// signal that survives homonyms when a head binds to a declaration.
    inherits_import_evidence: FxHashMap<(String, String), String>,
    /// Nearest enclosing type-kind ancestor: source_qname → enclosing_type_qname.
    enclosing_type: FxHashMap<String, String>,
    /// Identity twin: source row id → enclosing type's row id.
    enclosing_type_by_id: FxHashMap<i64, i64>,
    /// Nearest enclosing namespace/module ancestor: source_qname → enclosing_ns_qname.
    enclosing_namespace: FxHashMap<String, String>,
    /// Type-alias targets, keyed by both qualified and simple name. Populated
    /// from `ParsedFile::alias_targets`; consulted by `engine::alias::expand`.
    /// Every type-expression component is pre-interned into the arena at
    /// map-build time so `expand` reads TypeIds directly without re-interning.
    alias_target: FxHashMap<String, AliasTargetIds>,
    /// Type-alias targets keyed by the alias's declaration SYMBOL ID — the
    /// collision-free counterpart of `alias_target`. Two sibling `type Logger = …`
    /// aliases share the bare name key (last writer wins); their ids do not, so a
    /// use site that resolves `Logger` to a specific declaration expands the right
    /// target. See `alias_target_by_id`.
    alias_target_by_id: FxHashMap<i64, AliasTargetIds>,
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
    /// Manifest-declared dependency names, per package + union. Backs
    /// `is_declared_dependency` — cause-attribution evidence only.
    declared_deps: super::declared_deps::DeclaredDeps,
    /// Manifest-declared implicit/global namespace imports (`<ImplicitUsings>`,
    /// `<Using Include>`), per package and workspace-wide. Backs
    /// `SymbolLookup::implicit_wildcard_namespaces`.
    implicit_namespaces_by_pkg: FxHashMap<i64, Vec<String>>,
    implicit_namespaces_global: Vec<String>,
    /// Per-package Cargo dependency renames: (alias, target_package_name),
    /// snapshot of the cargo manifest's dep_renames. Per-consumer only.
    dep_renames_by_pkg: FxHashMap<i64, Vec<(String, String)>>,
    /// Symbols grouped by their owning workspace package id, for package-scoped
    /// lookups. Built from each symbol's `package_id` during ingest.
    by_package: FxHashMap<i64, Vec<Symbol>>,
    /// Ambient-scope symbols keyed by simple name: globals a package contributes
    /// without an import. Populated from symbols whose qualified name lives under
    /// the conventional ambient-scope namespace — the marker an ecosystem stamps
    /// on a `declare global` name or test-framework global at materialization.
    ambient_scope: FxHashMap<String, Vec<Symbol>>,
    /// Cross-package re-export aliases: `{importing_module}.{name}` → the
    /// declaration Symbol ingested under the DECLARING package's own prefix.
    /// A barrel's re-export binding carries no type of its own; the alias
    /// leads `all_by_qualified_name` (kind-gated callers keep the binding as
    /// fallback) and backs `reexport_alias_target` for chain-root typing.
    /// `by_qualified_name` stays alias-free: its single slot feeds
    /// type-derivation contexts where the binding symbol itself is the
    /// correct referent. Populated by `apply_external_reexport_aliases`.
    reexport_alias: FxHashMap<String, Symbol>,
    /// Cross-language external-declaration visibility: which languages'
    /// EXTERNAL declarations a file of a given language may bind by name.
    pub(super) ext_langs: ExtLangVisibility,
    /// Shared workspace type arena — the same one threaded through extractors.
    pub(super) arena: Arc<TypeArena>,
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
        symbol_id_map: &SymbolIds,
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
        symbol_id_map: &SymbolIds,
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
        self.module_specifier.snapshot_manifests(ctx);
        // One entry per isolated package (so an alias-less package declines rather
        // than borrowing the global set) plus the workspace-wide fallback. The
        // `AliasedImportRule` consults these via `resolve_path_alias`.
        for (&pkg_id, manifests) in &ctx.by_package {
            let aliases = manifests
                .get(&ManifestKind::Npm)
                .map(|m| m.path_aliases.clone())
                .unwrap_or_default();
            self.path_aliases_by_pkg.insert(pkg_id, aliases);
            let renames = manifests
                .get(&ManifestKind::Cargo)
                .map(|m| m.dep_renames.clone())
                .unwrap_or_default();
            self.dep_renames_by_pkg.insert(pkg_id, renames);
            if let Some(usings) = manifests
                .get(&ManifestKind::NuGet)
                .map(|m| m.global_usings.clone())
                .filter(|u| !u.is_empty())
            {
                self.implicit_namespaces_by_pkg.insert(pkg_id, usings);
            }
        }
        if let Some(npm) = ctx.manifests.get(&ManifestKind::Npm) {
            self.path_aliases_global = npm.path_aliases.clone();
        }
        if let Some(nuget) = ctx.manifests.get(&ManifestKind::NuGet) {
            self.implicit_namespaces_global = nuget.global_usings.clone();
        }
        self.ext_langs = ExtLangVisibility::snapshot(ctx);
        self.declared_deps = super::declared_deps::DeclaredDeps::snapshot(ctx);
    }

    /// The candidate-language codes whose EXTERNAL declarations a file of
    /// `lang` may bind by name; `None` disables the check.
    pub(crate) fn ext_lang_allowed(&self, lang: &str) -> Option<&FxHashSet<u16>> {
        self.ext_langs.allowed(lang)
    }

    /// Drop external-origin candidates whose file language is outside
    /// `allowed`, keeping the borrowed set when nothing is dropped.
    pub(crate) fn filter_ext_langs<'a>(
        &self,
        set: SymbolSet<'a>,
        allowed: Option<&FxHashSet<u16>>,
    ) -> SymbolSet<'a> {
        self.ext_langs.filter(set, allowed)
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
            pending_import_requalify: Vec::new(),
            reexport_map: FxHashMap::default(),
            module_entry: FxHashMap::default(),
            module_specifier: module_specifier::Context::default(),
            selector_to_qname: FxHashMap::default(),
            export_alias_by_module: FxHashMap::default(),
            inherits: FxHashMap::default(),
            inherits_args: FxHashMap::default(),
            inherits_arg_ids: FxHashMap::default(),
            inherits_by_id: FxHashMap::default(),
            inherits_args_by_pair: FxHashMap::default(),
            merge_groups: super::merge_canonical::MergeGroups::default(),
            merge_canonical: FxHashMap::default(),
            inherits_import_evidence: FxHashMap::default(),
            enclosing_type: FxHashMap::default(),
            enclosing_type_by_id: FxHashMap::default(),
            enclosing_namespace: FxHashMap::default(),
            alias_target: FxHashMap::default(),
            alias_target_by_id: FxHashMap::default(),
            workspace_pkg_by_declared_name: FxHashMap::default(),
            path_aliases_by_pkg: FxHashMap::default(),
            path_aliases_global: Vec::new(),
            declared_deps: super::declared_deps::DeclaredDeps::default(),
            implicit_namespaces_by_pkg: FxHashMap::default(),
            implicit_namespaces_global: Vec::new(),
            dep_renames_by_pkg: FxHashMap::default(),
            by_package: FxHashMap::default(),
            ambient_scope: FxHashMap::default(),
            reexport_alias: FxHashMap::default(),
            ext_langs: ExtLangVisibility::default(),
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
        symbol_id_map: &SymbolIds,
        ambient_qnames: &HashSet<String>,
    ) {
        // -----------------------------------------------------------------------
        // Phase A — structural indexes (Passes 1–4) over all files.
        // -----------------------------------------------------------------------
        // Extractor-set declared types, applied after the batch loop so the
        // merged value+type qname guard sees every type declaration:
        // `(qname, symbol id, declared TypeId)`.
        let mut pending_field_types: Vec<(String, i64, TypeId)> = Vec::new();
        let profiles = super::file_context::build_profiles();
        for pf in parsed {
            let merge_scope = profiles
                .get(pf.language.as_str())
                .map(|p| p.declaration_merging)
                .unwrap_or(crate::type_checker::profile::language_profile::MergeScope::None);
            let file_arc: Arc<str> = Arc::from(pf.path.as_str());

            // External file: record its parse language for the cross-language
            // visibility check. Languages without a code stay unrecorded and
            // therefore unfiltered.
            if pf.path.starts_with("ext:") {
                self.ext_langs.record_file(&file_arc, pf.language.as_str());
            }

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
                let Some(id) = symbol_id_map.id_of(&pf.path, sym_i, &sym.qualified_name)
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
                super::merge_canonical::record(
                    &mut self.merge_groups,
                    merge_scope,
                    &info,
                    &pf.path,
                    pf.package_id,
                );

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
                if let (Some(p_idx), Some(parent)) = (sym.parent_index, parent_sym) {
                    if let Some(parent_id) =
                        symbol_id_map.id_of(&pf.path, p_idx, &parent.qualified_name)
                    {
                        self.members_by_id.entry(parent_id).or_default().push(info.id);
                    }
                }

                // Type metadata from extractor-set TypeIds. Ref-derived
                // type_info fills absent slots in Phase B below. Return types
                // apply immediately; declared (field) types are deferred until
                // the whole batch's symbols are indexed, so the merged
                // value+type qname guard below sees every type declaration.
                if sym.declared_type.is_some() || sym.return_type.is_some() {
                    if let Some(type_id) = sym.declared_type {
                        pending_field_types.push((
                            sym.qualified_name.clone(),
                            info.id,
                            type_id,
                        ));
                    }
                    if let Some(type_id) = sym.return_type {
                        let ti = self
                            .type_info
                            .entry(sym.qualified_name.clone())
                            .or_insert_with(TypeInfo::default);
                        ti.return_type_id = Some(type_id);
                        // Mirror extractor-set types onto the id-keyed slot
                        // (info.id), so an id-driven read sees the same
                        // extractor-wins precedence as the qname slot.
                        let tid = self
                            .type_info_by_id
                            .entry(info.id)
                            .or_insert_with(TypeInfo::default);
                        tid.return_type_id = Some(type_id);
                    }
                }
            }

            // Pass 2 — enclosing_type / enclosing_namespace via parent_index.
            super::enclosing::build_enclosing_maps(
                pf,
                symbol_id_map,
                &mut self.enclosing_type,
                &mut self.enclosing_type_by_id,
                &mut self.enclosing_namespace,
            );

            // Pass 3 — inheritance from refs (Inherits / Implements edges).
            // The file's import bindings (name → module) disambiguate a parent
            // head against homonyms when the head binds to a declaration id.
            let import_modules = super::parent_resolution::import_evidence_of(pf);
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
                if let Some(module) = import_modules.get(parent_head.as_str()) {
                    self.inherits_import_evidence
                        .entry((child_sym.qualified_name.clone(), parent_head.clone()))
                        .or_insert_with(|| (*module).to_string());
                }
                if !parent_args.is_empty() {
                    let args: Vec<String> =
                        parent_args.iter().map(|a| a.trim().to_string()).collect();
                    let arg_ids: Vec<TypeId> =
                        args.iter().map(|a| self.arena.intern_type_str(a)).collect();
                    self.inherits_arg_ids
                        .entry(child_sym.qualified_name.clone())
                        .or_default()
                        .push((parent_head.clone(), arg_ids));
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
            // carries its scope prefix. Type-expression components are interned
            // once here so expand reads TypeIds directly.
            for (name, target) in &pf.alias_targets {
                let interned = intern_alias_target(&self.arena, target);
                self.alias_target.insert(name.clone(), interned);
                if let Some((_, simple)) = name.rsplit_once('.') {
                    self.alias_target
                        .entry(simple.to_string())
                        .or_insert_with(|| intern_alias_target(&self.arena, target));
                }
                // Id-keyed entry: the bare/qualified name collides across sibling
                // aliases (two `type Logger = …`), but the declaration id does not —
                // so a use site that resolves the alias to a specific declaration
                // expands its OWN target, not the last writer's.
                if let Some(&id) = symbol_id_map.by_key().get(&(pf.path.clone(), name.clone())) {
                    self.alias_target_by_id
                        .insert(id, intern_alias_target(&self.arena, target));
                }
            }
        }

        // Extractor-set declared types. A value whose qname is ALSO a type
        // declaration — the merged `declare var Date: DateConstructor` +
        // `interface Date` global pair — must not write its declared type into
        // either field slot: the qname slot types INSTANCES of the type, and
        // the id map cannot tell the two same-qname symbols apart. Same rule
        // as the ref-derived guard in Phase B; applied here after every file's
        // symbols are indexed so the check sees the whole batch.
        for (qname, id, type_id) in pending_field_types {
            let qname_owned_by_type = self
                .by_qname_all
                .get(&qname)
                .is_some_and(|cands| cands.iter().any(|c| is_type_like(&c.kind)));
            if qname_owned_by_type {
                continue;
            }
            let ti = self.type_info.entry(qname).or_insert_with(TypeInfo::default);
            ti.field_type_id = Some(type_id);
            let tid = self.type_info_by_id.entry(id).or_insert_with(TypeInfo::default);
            tid.field_type_id = Some(type_id);
        }

        // Module-entry map — package entries and declared ambient-module names
        // key module specifiers to the file re-export following starts from.
        // Runs after Pass 4 so `reexport_map` is complete. See `module_entry`.
        super::module_entry::populate(&mut self.module_entry, &self.reexport_map, parsed);

        // Declaration merging: fold the id-keyed structural indexes onto each
        // merge set's canonical id. Runs per batch; idempotent.
        self.merge_canonical = super::merge_canonical::compute(&self.merge_groups);
        super::merge_canonical::apply(
            &self.merge_canonical,
            &mut self.members_by_id,
            &mut self.inherits_by_id,
            &mut self.inherits_args_by_pair,
            &mut self.enclosing_type_by_id,
        );
        super::merge_canonical::fold_type_info(&self.merge_canonical, &mut self.type_info_by_id);

        self.module_specifier.file_paths.extend(module_specifier::internal_file_paths(parsed));

        // Selector → class qname. Backs SymbolLookup::selector_qname, which
        // SelectorMapRule consults to bind an Angular/CSS selector ref (`<nb-card>`,
        // `nbButton`) to its decorated class. An element selector (`nb-card`) keys on
        // its tag; an attribute directive (`button[nbButton],a[nbButton]`) keys on
        // each attribute name — the template binds a directive by its attribute, not
        // the element qualifier. First-writer-wins (a duplicate selector is an
        // Angular error).
        fn selector_binding_keys(raw: &str) -> Vec<String> {
            let mut keys = Vec::new();
            for part in raw.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let mut had_attr = false;
                let mut rest = part;
                while let Some(open) = rest.find('[') {
                    let Some(close) = rest[open..].find(']') else {
                        break;
                    };
                    let attr = rest[open + 1..open + close].trim();
                    // `[type=button]` / `[attr^="v"]` → the attribute NAME only.
                    let attr = attr
                        .split(['=', '~', '|', '^', '$', '*'])
                        .next()
                        .unwrap_or(attr)
                        .trim();
                    if !attr.is_empty() {
                        keys.push(attr.to_string());
                        had_attr = true;
                    }
                    rest = &rest[open + close + 1..];
                }
                if !had_attr {
                    let tag: String = part
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                        .collect();
                    if !tag.is_empty() {
                        keys.push(tag);
                    }
                }
            }
            keys
        }
        for pf in parsed {
            for (selector, class_qname) in &pf.component_selectors {
                for key in selector_binding_keys(selector) {
                    self.selector_to_qname
                        .entry(key)
                        .or_insert_with(|| class_qname.clone());
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

        // Import-scoped annotation heads. Each file's bare-headed type slots
        // requalify to the imports that lexically bind them. Collected per
        // batch, applied on EVERY ingest — a candidate the current batch
        // doesn't materialize is retried when a later external batch does.
        for pf in parsed {
            if let Some(pending) = import_qualify::collect_pending(pf, symbol_id_map) {
                self.pending_import_requalify.push(pending);
            }
        }
        import_qualify::apply_pending(
            &mut self.pending_import_requalify,
            &self.arena,
            &self.by_qname,
            &self.by_file,
            &mut self.type_info_by_id,
        );

        // Id-keyed inherits, derived from the now-complete `inherits` (child qname
        // → parent head string) + `by_qname`. Each edge resolves to specific
        // symbol ids so the chain walker climbs supertypes by identity.
        self.rebuild_inherits_by_id();
    }

    /// Merge ladder-RESOLVED inheritance edges into the climb map — see `parent_resolution::apply_resolved`.
    pub(crate) fn apply_resolved_inherits(&mut self, pairs: impl IntoIterator<Item = (i64, i64)>) {
        let pairs: Vec<(i64, i64)> = pairs.into_iter().collect();
        super::parent_resolution::apply_resolved(&mut self.inherits_by_id, pairs.iter().copied());
        super::parent_resolution::attach_edge_args(
            &pairs,
            &self.by_id,
            &self.inherits_arg_ids,
            &mut self.inherits_args_by_pair,
        );
    }

    /// Rebuild `inherits_by_id` from the string-keyed `inherits` map, the
    /// captured import evidence, and the fully-built symbol indexes. Rebuilt
    /// in full (not incrementally) because `ingest` / `ingest_from_db` each
    /// run over the cumulative symbol set. Resolution ranking lives in
    /// `engine/parent_resolution`.
    fn rebuild_inherits_by_id(&mut self) {
        let (by_id, args_by_pair) = super::parent_resolution::rebuild_inherits_by_id(
            self,
            &self.inherits,
            &self.inherits_import_evidence,
            &self.inherits_arg_ids,
        );
        self.inherits_by_id = by_id;
        self.inherits_args_by_pair.extend(args_by_pair);
    }

    /// Merge TypeScript module-augmentation supertypes into the augmented
    /// interface. A `declare module 'M' { interface I extends S {} }` in one
    /// package declares its `I` under THAT package, disconnected from the `I`
    /// module `M` exports — which is the type a value of `M`'s API carries.
    /// Resolve the target through `M`'s re-export and graft the augmentation's
    /// supertypes onto it, so a member declared on `S` resolves on the receiver.
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

    /// Register cross-package re-export aliases. Each tuple is
    /// `(alias_qname, target_qname, target_file)`: `alias_qname` is the qname
    /// the importing module's surface binds (`{module}.{name}`), resolved to
    /// the declaration ingested under `target_qname` — preferring the
    /// candidate from `target_file` when the qname is declared in several
    /// files. The declaration keeps its single symbol identity; the alias
    /// only adds a lookup key consulted by `all_by_qualified_name` and
    /// `reexport_alias_target`. First writer wins; a pair whose target is
    /// not materialized is skipped.
    pub(crate) fn apply_external_reexport_aliases(&mut self, aliases: &[(String, String, String)]) {
        for (alias_qname, target_qname, target_file) in aliases {
            if self.reexport_alias.contains_key(alias_qname) {
                continue;
            }
            let Some(candidates) = self.by_qname_all.get(target_qname) else {
                continue;
            };
            let sym = candidates
                .iter()
                .find(|s| &*s.file_path == target_file.as_str())
                .or_else(|| candidates.first());
            if let Some(sym) = sym {
                if sym.qualified_name != *alias_qname {
                    self.reexport_alias.insert(alias_qname.clone(), sym.clone());
                }
            }
        }
    }

    /// The qualified name of the interface that `module` exports as `iface` —
    /// resolved through a re-export declared in a file belonging to `module`
    /// (`export { I } from '@scope/pkg'` → the `@scope/pkg.I` interface). Falls
    /// back to an interface the module declares directly under its own package.
    /// `None` when neither names an indexed symbol — the augmentation then merges
    /// nowhere rather than guessing a same-named interface in an unrelated package.
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
        symbol_id_map: &SymbolIds,
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
            // The `_primitive` coverage sentinel (emitted by the file-level
            // type-identifier scan for primitives / literals / structured
            // annotations, always attributed to a fixed symbol index — never
            // necessarily THIS symbol's own) carries no real type. `pipeline.rs`
            // already excludes it from edges/unresolved_refs; this fallback must
            // honor the same exclusion, or a stray sentinel from an unrelated
            // nested annotation (attributed to the same index) wins as `last()`
            // and clobbers this symbol's real return/field type.
            if r.target_name == "_primitive" {
                continue;
            }
            if r.source_symbol_index >= type_refs_by_sym.len() {
                continue;
            }
            // The file-level type-identifier coverage scan attributes every ref
            // it emits to a single fixed symbol index rather than the
            // declaration the annotation actually belongs to — its own doc
            // contract only guarantees "a ref at the correct line", not correct
            // attribution. A ref whose line falls outside the attributed
            // symbol's own [start_line, end_line] span is one of these: it
            // belongs to an unrelated, possibly much later, declaration and
            // must not compete for that symbol's `last()` return/field-type
            // fallback. Only enforced when the symbol carries a real
            // (non-degenerate) span — a single-point span carries no scoping
            // signal to check a ref's line against.
            let owner = &pf.symbols[r.source_symbol_index];
            if owner.end_line > owner.start_line
                && (r.line < owner.start_line || r.line > owner.end_line)
            {
                continue;
            }
            type_refs_by_sym[r.source_symbol_index]
                .push((r.target_name.as_str(), r.module.as_deref()));
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
                        if let Some((_, _, fid)) = derived {
                            // Qname slot — first-writer-wins.
                            let ti =
                                self.type_info.entry(sym.qualified_name.clone()).or_default();
                            if ti.field_type_id.is_none() {
                                ti.field_type_id = Some(fid);
                            }
                            // Id slot — keyed by this symbol's id, kept distinct from a
                            // same-qname first-winner so a caller holding the resolved
                            // id reads THIS field's type.
                            if let Some(&id) = symbol_id_map
                                .by_key()
                                .get(&(pf.path.clone(), sym.qualified_name.clone()))
                            {
                                let tid = self.type_info_by_id.entry(id).or_default();
                                if tid.field_type_id.is_none() {
                                    tid.field_type_id = Some(fid);
                                }
                            }
                        }
                    }

                    // A callable-typed property (`fn: () => Mock`) yields its
                    // function's return type when CALLED — captured here so
                    // `obj.fn().member` chains continue past the call.
                    let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                    if ti.return_type_id.is_none() {
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
                        }
                    }
                }
                SymbolKind::TypeAlias => {
                    if let Some(&(first, _)) = type_refs.first() {
                        let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                        if ti.field_type_id.is_none() {
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
                        if let Some(obj) = sig_rt
                            .as_deref()
                            .filter(|s| {
                                let t = s.trim();
                                t.starts_with('{') && t.ends_with('}')
                            })
                            .filter(|_| {
                                self.by_qname
                                    .contains_key(&format!("{}$Ret", sym.qualified_name))
                            })
                        {
                            // An inline object-type return (`fn(): { a: A; b: B }`) has a
                            // synthesized member-bearing `{fn}$Ret` interface (from the
                            // object-literal return). Type its members from the annotation
                            // and route the return THROUGH `$Ret`, so member access /
                            // destructuring of the call result resolves on the real members
                            // rather than a member-less inline-object-type string.
                            let synth = format!("{}$Ret", sym.qualified_name);
                            for (mname, mtype) in parse_object_type_members(obj) {
                                let mqname = format!("{synth}.{mname}");
                                if !self.by_qname.contains_key(&mqname) {
                                    continue;
                                }
                                let resolved = resolve_type_name_in_scope(
                                    &mtype,
                                    sym.scope_path.as_deref(),
                                    &self.by_qname,
                                );
                                let rid = self.arena.intern_type_str(&resolved);
                                let mti = self.type_info.entry(mqname.clone()).or_default();
                                if mti.field_type_id.is_none() {
                                    mti.field_type_id = Some(rid);
                                }
                                if let Some(&mid) =
                                    symbol_id_map.by_key().get(&(pf.path.clone(), mqname))
                                {
                                    let mtid = self.type_info_by_id.entry(mid).or_default();
                                    if mtid.field_type_id.is_none() {
                                        mtid.field_type_id = Some(rid);
                                    }
                                }
                            }
                            let sid = self.arena.class(&synth);
                            // Override both stores: the extractor mirrored the inline
                            // object-type return onto both the qname and id slots
                            // (above), and the id slot — read first by the resolver and
                            // COALESCEd over the qname slot on persist — would otherwise
                            // keep the member-less string. `set_return_both` forces both.
                            self.set_return_both(&sym.qualified_name, synth.clone(), sid);
                            Some((synth, Some(sid)))
                        } else if matches!(sig_rt.as_deref(), Some("this") | Some("Self")) {
                            // A fluent self-returning method (`m(fn: P): this`)
                            // yields the receiver. Its `this` return emits no
                            // TypeRef, so the `type_refs.last()` arm below would
                            // otherwise pick the LAST PARAMETER type — capture
                            // `this` verbatim so the chain walker's self-head rebind
                            // returns the receiver instead.
                            let rid = self.arena.intern_type_str("this");
                            Some(("this".to_string(), Some(rid)))
                        } else if let Some(rt) = sig_rt.as_deref().filter(|s| {
                            let t = s.trim();
                            t.starts_with('[') && t.ends_with(']')
                        }) {
                            // A tuple return (`useState(): [S, Dispatch<…>]`) interns
                            // directly as `Type::Tuple`; `parse_type_head_and_args`
                            // (the `sig_generic` arm) would mis-read the `[A, B]` as a
                            // generic application and drop the positional structure a
                            // destructure binding indexes.
                            let rid = self.arena.intern_type_str(rt);
                            Some((self.arena.format_type(rid), Some(rid)))
                        } else if let Some((tb, fb)) =
                            sig_rt.as_deref().and_then(parse_top_level_conditional)
                        {
                            // A conditional return (`… extends … ? A : B`) is
                            // undecidable here; carry its BRANCHES, not its check —
                            // the member-lookup semantics an undecidable conditional
                            // ALIAS already gets. A `never` branch carries no members
                            // and is dropped; both live branches join as an
                            // Intersection so a member declared on whichever branch
                            // applies still resolves. Each branch's head is
                            // scope-qualified like every other return head, so a
                            // package-local name binds its declaration.
                            let intern_branch = |branch: &str| -> TypeId {
                                let (head, args) = parse_type_head_and_args(branch);
                                let head_is_name = !head.is_empty()
                                    && head.chars().all(|c| {
                                        c.is_alphanumeric() || c == '_' || c == '.'
                                    });
                                if !head_is_name {
                                    return self.arena.intern_type_str(branch);
                                }
                                let resolved = resolve_type_name_in_scope(
                                    head,
                                    sym.scope_path.as_deref(),
                                    &self.by_qname,
                                );
                                let args: Vec<String> =
                                    args.iter().map(|s| s.to_string()).collect();
                                intern_head_and_args(&self.arena, &resolved, &args)
                            };
                            let t_id = (tb.trim() != "never").then(|| intern_branch(&tb));
                            let f_id = (fb.trim() != "never").then(|| intern_branch(&fb));
                            let rid = match (t_id, f_id) {
                                (Some(t), Some(f)) => {
                                    self.arena.intern(Type::Intersection(vec![t, f]))
                                }
                                (Some(t), None) => t,
                                (None, Some(f)) => f,
                                (None, None) => self.arena.intern_type_str("never"),
                            };
                            Some((self.arena.format_type(rid), Some(rid)))
                        } else if let Some((head, args)) = sig_generic {
                            let resolved = resolve_type_name_in_scope(
                                &head,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            let rid = intern_head_and_args(&self.arena, &resolved, &args);
                            Some((resolved, Some(rid)))
                        } else if let Some(&(last, _)) = type_refs.last().filter(|&&(last, _)| {
                            // The trailing TypeRef is the return type only when it
                            // NAMES the signature's return. A return whose annotation
                            // emits no TypeRef — a bare generic param (`(t: Token<T>):
                            // T`), a conditional, an indexed access — leaves the last
                            // PARAMETER's ref trailing, and recording that as the
                            // return types every call site with an argument type.
                            match sig_rt.as_deref().map(str::trim) {
                                Some(rt) => {
                                    last == rt || {
                                        let (head, _) = parse_type_head_and_args(rt);
                                        !head.is_empty() && last == head
                                    }
                                }
                                // No annotated return: an inferred-return callable's
                                // last TypeRef is its return only when the signature
                                // carries no params at all (otherwise it is the last
                                // parameter type).
                                None => sym
                                    .signature
                                    .as_deref()
                                    .and_then(parse_param_types_from_signature)
                                    .map_or(true, |params| params.is_empty()),
                            }
                        }) {
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
                    if let Some((_, rid)) = computed {
                        // Qname slot — first-writer-wins (unchanged).
                        let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                        if ti.return_type_id.is_none() {
                            ti.return_type_id = rid;
                        }
                        // Id slot — keyed by this symbol's id; no qname collision.
                        if let Some(id) =
                            symbol_id_map.id_of(&pf.path, sym_idx, &sym.qualified_name)
                        {
                            let tid = self.type_info_by_id.entry(id).or_default();
                            if tid.return_type_id.is_none() {
                                tid.return_type_id = rid;
                            }
                        }
                    }
                }
                SymbolKind::Class => {
                    // Constructor call yields the class itself.
                    let class_id = self.arena.class(&sym.qualified_name);
                    let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                    if ti.return_type_id.is_none() {
                        ti.return_type_id = Some(class_id);
                    }
                    if let Some(id) =
                        symbol_id_map.id_of(&pf.path, sym_idx, &sym.qualified_name)
                    {
                        let tid = self.type_info_by_id.entry(id).or_default();
                        if tid.return_type_id.is_none() {
                            tid.return_type_id = Some(class_id);
                        }
                    }
                }
                _ => {}
            }
        }

        // Generic-param extraction from signatures for types and callables.
        // The clause locator is name-anchored: the declaration name's own
        // bracket group, never a return-type application's argument list.
        for (sym_idx, sym) in pf.symbols.iter().enumerate() {
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
            let gparams = signature_generic_params(sig, &sym.name);
            if gparams.is_empty() {
                continue;
            }
            let mut param_ids = Vec::with_capacity(gparams.len());
            let mut default_ids = Vec::with_capacity(gparams.len());
            for (n, b, d) in gparams {
                let bound = b.as_deref().map(|s| self.arena.intern_type_str(s));
                let gp_id = self.arena.intern_generic(GenericParamData {
                    name: n,
                    owner_symbol_index: 0,
                    bound,
                });
                param_ids.push(gp_id);
                default_ids.push(d.map(|s| self.arena.intern_type_str(&s)));
            }
            // Key on both simple name and qname so callers
            // using either form get the params.
            for key in [&sym.name, &sym.qualified_name] {
                let ti = self.type_info.entry(key.clone()).or_default();
                if ti.generic_param_ids.is_empty() {
                    ti.generic_param_ids = param_ids.clone();
                    ti.generic_param_default_ids = default_ids.clone();
                }
            }
            // Id slot — immune to the qname collision that collapses a
            // name like `useQuery` (shared by several packages and doc
            // fences) to a single first-winner in the qname slot, so
            // the real source declarations lose their params. The
            // (path,qname) map resolves an overload set to its
            // implementation id, co-locating the overloads' params with
            // the return the resolver reads there.
            if let Some(id) =
                symbol_id_map.id_of(&pf.path, sym_idx, &sym.qualified_name)
            {
                let tid = self.type_info_by_id.entry(id).or_default();
                if tid.generic_param_ids.is_empty() {
                    tid.generic_param_ids = param_ids.clone();
                    tid.generic_param_default_ids = default_ids.clone();
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
            let Some(field_type_id) = resolve_module_exported_value_type(
                &p.module,
                &p.key,
                &p.typed_qname,
                &self.export_alias_by_module,
                &self.by_qname,
                &self.type_info,
                &self.arena,
            ) else {
                continue;
            };
            let ti = self.type_info.entry(p.typed_qname).or_default();
            if ti.field_type_id.is_none() {
                ti.field_type_id = Some(field_type_id);
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
        // (fn_qname, candidate_return_type, candidate_return_type_id)
        let mut candidates: Vec<(String, String, Option<TypeId>)> = Vec::new();

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
                    .and_then(|ti| ti.return_type_id)
                    .is_some()
                {
                    continue;
                }
                let param_qname = format!("{}.{}", fn_sym.qualified_name, ident);
                let Some(param_ti) = self.type_info.get(&param_qname) else {
                    continue;
                };
                let Some(fid) = param_ti.field_type_id else {
                    continue;
                };
                let ty = self.arena.format_type(fid);
                if ty.is_empty() || ty.eq_ignore_ascii_case("unknown") {
                    continue;
                }
                // An unbound type variable of the function is not an inference.
                let is_generic_param = self
                    .type_info
                    .get(&fn_sym.qualified_name)
                    .map(|ti| {
                        ti.generic_param_ids
                            .iter()
                            .any(|&id| self.arena.generic_param(id).name == ty)
                    })
                    .unwrap_or(false);
                if is_generic_param {
                    continue;
                }
                candidates.push((fn_sym.qualified_name.clone(), ty, param_ti.field_type_id));
            }
        }

        for (qname, ty, ty_id) in agree_inferred_returns(candidates) {
            // Re-check the slot: a same-batch candidate ordering, or a slot a
            // concurrent function already filled, must not be overwritten.
            let ti = self.type_info.entry(qname).or_default();
            if ti.return_type_id.is_some() {
                continue;
            }
            ti.return_type_id =
                ty_id.or_else(|| Some(intern_head_and_args(&self.arena, &ty, &[])));
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
    /// external function (`ReturnType<typeof f>` where `f` is an external
    /// package's) sees that function's return type.
    pub(crate) fn resolve_wrapper_return_types(&mut self, parsed: &[ParsedFile]) {
        let mut rewrites: Vec<(String, TypeId)> = Vec::new();
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
                    .and_then(|ti| ti.return_type_id)
                    .map(|id| self.arena.format_type(id))
                    .filter(|rt| rt.contains("typeof"))
                else {
                    continue;
                };
                let id = self.arena.intern_type_str(&rt);
                if let Some(resolved) =
                    super::chain::resolve_return_type_extraction(id, self, &self.arena, &file_ctx)
                {
                    rewrites.push((sym.qualified_name.clone(), resolved));
                }
            }
        }
        for (qname, id) in rewrites {
            if let Some(ti) = self.type_info.get_mut(&qname) {
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
                let Some(ret_id) = super::chain::callee_return_type_in_scope(
                    self,
                    &self.arena,
                    &file_ctx,
                    &call_ref.target_name,
                    &fn_sym.qualified_name,
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
                    self.alias_target.insert(
                        union_name.clone(),
                        intern_alias_target(&self.arena, &AliasTarget::Union(ret_branches)),
                    );
                    let id = self.arena.class(&union_name);
                    (union_name, id)
                };
                self.set_return_both(&qname, ret_str, ret_id);
                self.mirror_ret_interface_member(&qname, ret_id);
                continue;
            }
            // No object-literal builder branch: keep a single agreed non-synthetic
            // return, not overriding an existing slot (the wrapper-hook case).
            if self
                .type_info
                .get(&qname)
                .and_then(|ti| ti.return_type_id)
                .is_some()
            {
                continue;
            }
            if distinct.len() == 1 {
                let (ty, ty_id, type_args) = distinct.into_iter().next().unwrap();
                let rid = ty_id.unwrap_or_else(|| intern_head_and_args(&self.arena, &ty, &type_args));
                let ti = self.type_info.entry(qname.clone()).or_default();
                ti.return_type_id = Some(rid);
                self.mirror_ret_interface_member(&qname, rid);
            }
        }
    }

    /// Mirror a wrapped method's inferred return onto its `{scope}$Ret.{member}`
    /// synthetic sibling, when one exists. The object-literal-return member
    /// synthesis (`{fn}$Ret` in `parse_file.rs`) creates that sibling carrying
    /// no return type of its own — so a factory method that itself wraps
    /// another call (`createNullLogger() { return { with() { return
    /// createLogger() } } }`) would type `createNullLogger.with` but leave
    /// `createNullLogger$Ret.with` (what chain-walking a call's inferred
    /// receiver actually reads) untyped, stopping a further chain
    /// (`nl.with().log()`) at `with`.
    fn mirror_ret_interface_member(&mut self, qname: &str, ret_id: TypeId) {
        let Some(dot) = qname.rfind('.') else {
            return;
        };
        let mirror_qname = format!("{}$Ret.{}", &qname[..dot], &qname[dot + 1..]);
        if self.by_qname.contains_key(&mirror_qname) {
            self.set_return_both(&mirror_qname, String::new(), ret_id);
        }
    }

    /// Write a resolved return type to BOTH stores: the qname slot and every
    /// same-qname symbol's id slot, overriding what is there. The resolver reads
    /// the id slot (`return_type_id_of`) BEFORE the qname slot, and persist
    /// COALESCEs the id store over the qname store — so a qname-only write is
    /// shadowed by a stale id-keyed value.
    fn set_return_both(&mut self, qname: &str, _ret_str: String, ret_id: TypeId) {
        let ids: Vec<i64> = self
            .by_qname_all
            .get(qname)
            .map(|v| v.iter().map(|s| s.id).collect())
            .unwrap_or_default();
        let ti = self.type_info.entry(qname.to_string()).or_default();
        ti.return_type_id = Some(ret_id);
        for id in ids {
            let tid = self.type_info_by_id.entry(id).or_default();
            tid.return_type_id = Some(ret_id);
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
    ///
    /// `readonly res = await fetch(url)` needs one more step: the initializer's
    /// raw return type is the async wrapper itself (`Promise<Response>`), not
    /// what `await` yields (`Response`). `flow_binding_await` — populated by
    /// the same per-file flow pass that seeds forward-inference locals — names
    /// which field/variable symbols were bound from an `await`-ed initializer;
    /// when the field's own index is in that set, one async-wrapper layer is
    /// peeled per `profiles[pf.language].async_wrappers` before the field's type
    /// is recorded, mirroring the peel `resolve_one_file`'s binding seed applies.
    /// The instance type a `new X(...)` initializer builds. `X` may be a VALUE
    /// in an enclosing scope — a `Ctor: typeof C` parameter — whose
    /// constructor type's instance is what `new` yields; a value typed by a
    /// bare instance name (the `typeof C` capture shape) carries that instance
    /// directly. Only when no in-scope value carries the name does `X`
    /// scope-resolve as the class itself. The global by-name pool is never
    /// consulted for the value form — an unrelated package's same-named value
    /// must not type this binding.
    fn instantiated_type(&self, name: &str, scope_path: Option<&str>, file: &str) -> TypeId {
        let mut scope = scope_path.unwrap_or("");
        while !scope.is_empty() {
            let qn = format!("{scope}.{name}");
            // Sibling packages repeat scope qnames (`useBaseQuery.Observer` in
            // four framework adapters) — the value in THIS file is the one the
            // initializer names, so a same-file candidate wins over the qname
            // slot's first-winner.
            let cands = self.all_by_qualified_name(&qn);
            let s = cands
                .iter()
                .find(|s| &*s.file_path == file)
                .or_else(|| cands.iter().next());
            if let Some(s) = s {
                if matches!(
                    s.kind.as_str(),
                    "parameter" | "property" | "field" | "variable" | "constant"
                ) {
                    let ft = self
                        .type_info_by_id
                        .get(&s.id)
                        .and_then(|ti| ti.field_type_id)
                        .or_else(|| self.type_info.get(&qn).and_then(|ti| ti.field_type_id));
                    if let Some(ft) = ft {
                        return match self.arena.get(ft) {
                            Type::Constructor(inner) => inner,
                            _ => ft,
                        };
                    }
                }
            }
            scope = match scope.rfind('.') {
                Some(i) => &scope[..i],
                None => "",
            };
        }
        let resolved = resolve_type_name_in_scope(name, scope_path, &self.by_qname);
        self.arena.class(&resolved)
    }

    pub(crate) fn infer_field_init_types(
        &mut self,
        parsed: &[ParsedFile],
        profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
    ) {
        let mut updates: Vec<(String, Option<i64>, TypeId)> = Vec::new();
        for pf in parsed.iter().filter(|p| !p.path.starts_with("ext:")) {
            let async_wrappers = profiles
                .get(pf.language.as_str())
                .map(|p| p.async_wrappers)
                .unwrap_or_default();
            let file_ctx = init_pass_file_ctx(pf);
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
                // The guard and the write are keyed by THIS file's declaration
                // id — sibling packages repeat qnames (`useBaseQuery.observer`
                // in four adapters), and a shared qname slot would let the
                // first package processed suppress and mis-type the rest.
                let field_id = self
                    .all_by_qualified_name(&field.qualified_name)
                    .iter()
                    .find(|s| &*s.file_path == pf.path.as_str())
                    .map(|s| s.id);
                if let Some(id) = field_id {
                    if self
                        .type_info_by_id
                        .get(&id)
                        .and_then(|ti| ti.field_type_id)
                        .is_some()
                    {
                        continue;
                    }
                } else if self
                    .type_info
                    .get(&field.qualified_name)
                    .and_then(|ti| ti.field_type_id)
                    .is_some()
                {
                    continue;
                }
                let ty_id = match r.kind {
                    // `new X()` — the field is the INSTANCE `X` builds. `X`
                    // may name a VALUE in scope (a constructor-typed
                    // parameter), whose constructor type carries the instance;
                    // otherwise it scope-resolves as the class itself.
                    EdgeKind::Instantiates => Some(self.instantiated_type(
                        &r.target_name,
                        field.scope_path.as_deref(),
                        &pf.path,
                    )),
                    // `call(...)` — the field is the callee's return, with the
                    // call's argument types bound into any generic parameter
                    // the return names.
                    _ => super::chain::init_call_return_type(self, &self.arena, &file_ctx, r),
                };
                let Some(id) = ty_id else {
                    continue;
                };
                let id = if pf.flow.flow_binding_await.contains(&field_idx) {
                    super::pipeline::unwrap_async_yield_id(id, &self.arena, async_wrappers)
                } else {
                    id
                };
                let s = self.arena.format_type(id);
                if s.is_empty() || s.eq_ignore_ascii_case("unknown") {
                    continue;
                }
                updates.push((field.qualified_name.clone(), field_id, id));
            }
        }
        for (qname, sym_id, id) in updates {
            if let Some(sym_id) = sym_id {
                let ti = self.type_info_by_id.entry(sym_id).or_default();
                if ti.field_type_id.is_none() {
                    ti.field_type_id = Some(id);
                }
            }
            // The qname slot stays a fallback for name-keyed readers; first
            // writer wins there, the id slot carries each declaration's own.
            let ti = self.type_info.entry(qname).or_default();
            if ti.field_type_id.is_none() {
                ti.field_type_id = Some(id);
            }
        }
    }

    /// Type a CHAIN-initialized binding (`const c = base.with(x).use(cb)`)
    /// from its initializer chain's final yield, walked with the full member
    /// walker. Runs after `infer_field_init_types` so a chain rooted on a
    /// single-init binding (`base = make()`) reads that binding's type;
    /// bindings are processed in declaration order within a file so a later
    /// chain roots on an earlier chain's result.
    ///
    /// The trigger is the chain-bearing TypeRef the extractor emits ONLY for
    /// an annotation-less initializer, so the yield is authoritative: it
    /// OVERWRITES the id-keyed slot, where a name-derived pass could only
    /// have recorded a type read off the initializer's parts.
    pub(crate) fn infer_chain_init_types(
        &mut self,
        parsed: &[ParsedFile],
        profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
    ) {
        for pf in parsed.iter().filter(|p| !p.path.starts_with("ext:")) {
            let Some(profile) = profiles.get(pf.language.as_str()) else {
                continue;
            };
            let mut candidates: Vec<(usize, &crate::types::ExtractedRef)> = Vec::new();
            for r in &pf.refs {
                if r.kind != EdgeKind::TypeRef {
                    continue;
                }
                if r.chain.as_ref().map(|c| c.segments.len()).unwrap_or(0) < 2 {
                    continue;
                }
                let Some(sym) = pf.symbols.get(r.source_symbol_index) else {
                    continue;
                };
                if !matches!(
                    sym.kind,
                    SymbolKind::Property | SymbolKind::Field | SymbolKind::Variable
                ) || sym.declared_type.is_some()
                {
                    continue;
                }
                candidates.push((r.source_symbol_index, r));
            }
            // Declaration order; one marker per binding (the first).
            candidates.sort_by_key(|(idx, r)| {
                let s = &pf.symbols[*idx];
                (s.start_line, s.start_col, r.byte_offset)
            });
            candidates.dedup_by_key(|(idx, _)| *idx);

            let file_ctx = init_pass_file_ctx(pf);
            let async_wrappers = profiles
                .get(pf.language.as_str())
                .map(|p| p.async_wrappers)
                .unwrap_or_default();
            for (field_idx, r) in candidates {
                let field = &pf.symbols[field_idx];
                let field_id = self
                    .all_by_qualified_name(&field.qualified_name)
                    .iter()
                    .find(|s| &*s.file_path == pf.path.as_str())
                    .map(|s| s.id);
                let yield_id = {
                    let ref_ctx = RefContext {
                        extracted_ref: r,
                        source_symbol: field,
                        scope_chain: build_scope_chain(field.scope_path.as_deref()),
                        file_package_id: pf.package_id,
                        source_symbol_id: field_id,
                    };
                    match super::chain::bind_member_access(&ref_ctx, &file_ctx, self, profile) {
                        Ok(res) => res.resolved_yield_type,
                        Err(_) => None,
                    }
                };
                let Some(id) = yield_id else {
                    continue;
                };
                let id = if pf.flow.flow_binding_await.contains(&field_idx) {
                    super::pipeline::unwrap_async_yield_id(id, &self.arena, async_wrappers)
                } else {
                    id
                };
                let s = self.arena.format_type(id);
                if s.is_empty() || s.eq_ignore_ascii_case("unknown") {
                    continue;
                }
                if let Some(sym_id) = field_id {
                    self.type_info_by_id.entry(sym_id).or_default().field_type_id = Some(id);
                }
                let ti = self.type_info.entry(field.qualified_name.clone()).or_default();
                if ti.field_type_id.is_none() {
                    ti.field_type_id = Some(id);
                }
            }
        }
    }
}

/// The import surface of one parsed file, as the `FileContext` the
/// initializer-typing passes hand to the chain walker.
fn init_pass_file_ctx(pf: &ParsedFile) -> FileContext {
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
    FileContext {
        file_path: pf.path.clone(),
        language: pf.language.clone(),
        imports,
        file_namespace: None,
    }
}

// ---------------------------------------------------------------------------
// SymbolLookup impl
// ---------------------------------------------------------------------------

// All flow-cache methods keep their no-op defaults: the per-file cache lives
// on the FileLookup overlay, never on the shared tree.
impl Compilation {
    /// Canonical id of a merge-set member; identity for everything else.
    fn canon_id(&self, id: i64) -> i64 {
        self.merge_canonical.get(&id).copied().unwrap_or(id)
    }

}

impl crate::indexer::resolve::engine::contract::FlowCacheLookup for Compilation {}

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
        let base = self
            .by_qname_all
            .get(qname)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty);
        // Bridged declaration first; the same-qname binding symbols stay as
        // fallbacks so a kind-gated caller loses nothing.
        match self.reexport_alias.get(qname) {
            Some(alias) => SymbolSet::Owned(std::iter::once(alias).chain(base.iter()).collect()),
            None => SymbolSet::Borrowed(base),
        }
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
        match self.members_by_id.get(&self.canon_id(parent_id)) {
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

    fn field_type_name(&self, _property_qname: &str) -> Option<&str> {
        None
    }

    fn return_type_name(&self, _method_qname: &str) -> Option<&str> {
        None
    }

    fn generic_params(&self, type_name: &str) -> Option<Vec<String>> {
        let ti = self.type_info.get(type_name)?;
        if ti.generic_param_ids.is_empty() {
            return None;
        }
        Some(
            ti.generic_param_ids
                .iter()
                .map(|&id| self.arena.generic_param(id).name)
                .collect(),
        )
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

    fn generic_params_of(&self, symbol_id: i64) -> Option<Vec<String>> {
        let ti = self.type_info_by_id.get(&symbol_id)?;
        if ti.generic_param_ids.is_empty() {
            return None;
        }
        Some(
            ti.generic_param_ids
                .iter()
                .map(|&id| self.arena.generic_param(id).name)
                .collect(),
        )
    }

    fn generic_param_defaults_of(&self, symbol_id: i64) -> Option<Vec<Option<String>>> {
        let ti = self.type_info_by_id.get(&symbol_id)?;
        if ti.generic_param_ids.is_empty() {
            return None;
        }
        Some(
            ti.generic_param_default_ids
                .iter()
                .map(|opt_id| opt_id.map(|id| self.arena.format_type(id)))
                .collect(),
        )
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

    fn resolve_module_from(&self, _source_file: &str, spec: &str) -> Option<&str> {
        // Bare package specifier → its indexed entry file. Relative specifiers are
        // resolved by the relative re-export path in `reexport_following`; returning
        // None here preserves that fallback.
        if super::support::is_relative_specifier(spec) {
            return None;
        }
        self.module_entry.get(spec).map(String::as_str)
    }

    fn resolve_module_via_language_resolver(
        &self, language: &str, source_file: &str, spec: &str,
    ) -> Option<String> {
        module_specifier::resolve_via_module_resolver(
            language, source_file, spec, self.package_id_for_file(source_file),
            &self.workspace_pkg_by_declared_name,
            self.module_specifier.go_module_path.as_deref(),
            &self.module_specifier.workspace_packages, &self.module_specifier.file_paths,
        )
    }

    fn in_module_from(&self, source_file: &str, spec: &str) -> SymbolSet<'_> {
        // A bare specifier resolves to its entry file's symbols; the None arm keeps
        // the legacy `in_file(spec)` fallback for already-file-shaped specifiers.
        match self.resolve_module_from(source_file, spec) {
            Some(path) => self.in_file(path),
            None => self.in_file(spec),
        }
    }

    fn resolve_external_reexport(&self, target: &str, _prefix: &str, module: &str) -> Option<i64> {
        // Drive the generic re-export walker from the package's indexed entry. The
        // calling rule re-checks edge-kind (`candidate_with_compatible_kind`), so an
        // accept-any gate here is sound.
        let entry = self.module_entry.get(module)?;
        super::support::follow_reexports(entry, target, EdgeKind::TypeRef, &|_, _| true, self, 0, &["index"])
            .map(|info| info.target_symbol_id)
    }

    fn reexport_alias_target(&self, qname: &str) -> Option<&Symbol> {
        self.reexport_alias.get(qname)
    }

    fn selector_qname(&self, raw_selector: &str) -> Option<&str> {
        self.selector_to_qname.get(raw_selector).map(String::as_str)
    }

    fn is_external_name(&self, _name: &str, _language: &str) -> bool {
        // External classification is a later phase; conservative false here.
        false
    }

    fn is_declared_dependency(&self, package_id: Option<i64>, spec: &str) -> bool {
        self.declared_deps.contains(package_id, spec)
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
        self.inherits_by_id.get(&self.canon_id(child_id)).cloned().unwrap_or_default()
    }

    fn parent_class_args(&self, child_head: &str, parent_head: &str) -> &[String] {
        self.inherits_args
            .get(child_head)
            .and_then(|edges| edges.iter().find(|(h, _)| h == parent_head))
            .map(|(_, args)| args.as_slice())
            .unwrap_or(&[])
    }

    fn parent_class_arg_ids_of(&self, child_id: i64, parent_id: i64) -> &[TypeId] {
        self.inherits_args_by_pair
            .get(&(self.canon_id(child_id), self.canon_id(parent_id)))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn parent_class_arg_ids(&self, child_head: &str, parent_head: &str) -> &[TypeId] {
        self.inherits_arg_ids
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

    fn enclosing_type_id_of(&self, source_symbol_id: i64) -> Option<i64> {
        self.enclosing_type_by_id
            .get(&self.canon_id(source_symbol_id))
            .copied()
    }

    fn canonical_decl_id(&self, id: i64) -> i64 {
        self.canon_id(id)
    }

    fn enclosing_namespace_qname(&self, source_qname: &str) -> Option<&str> {
        self.enclosing_namespace
            .get(source_qname)
            .map(|s| s.as_str())
    }

    fn alias_target(&self, name: &str) -> Option<&AliasTargetIds> {
        self.alias_target.get(name)
    }

    fn alias_target_by_id(&self, id: i64) -> Option<&AliasTargetIds> {
        self.alias_target_by_id.get(&id)
    }

    fn workspace_package_id(&self, specifier: &str) -> Option<i64> {
        // `::` (Rust's qualification separator) canonicalizes to `/` so a
        // deep `tantivy::schema` import peels the same way `@org/utils/sub`
        // does below.
        let normalized;
        let specifier: &str = if specifier.contains("::") {
            normalized = specifier.replace("::", "/");
            &normalized
        } else {
            specifier
        };
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

    fn implicit_wildcard_namespaces(&self, package_id: Option<i64>) -> &[String] {
        match package_id.and_then(|id| self.implicit_namespaces_by_pkg.get(&id)) {
            Some(per_pkg) => per_pkg.as_slice(),
            None => self.implicit_namespaces_global.as_slice(),
        }
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

    fn dep_rename(&self, consumer_pkg: Option<i64>, alias: &str) -> Option<&str> {
        let renames = self.dep_renames_by_pkg.get(&consumer_pkg?)?;
        renames.iter().find(|(a, _)| a == alias).map(|(_, pkg)| pkg.as_str())
    }

    fn package_id_for_file(&self, file_path: &str) -> Option<i64> {
        self.by_file.get(file_path).and_then(|v| v.first()).and_then(|s| s.package_id)
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
                    s.visibility, f.package_id, s.signature, s.containing_id, f.language \
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
                r.get::<_, String>(10)?,
            ))
        }) else {
            return id_map;
        };
        for (id, name, qname, kind, path, scope_path, visibility, package_id, signature, containing_id, language) in
            rows.flatten()
        {
            // Every DB symbol contributes to the id map, even ones the parsed
            // batch already owns.
            id_map.insert((path.clone(), qname.clone()), id);

            // External file: record its parse language for the cross-language
            // visibility check — same capture the full pass does in `ingest`.
            if path.starts_with("ext:") {
                self.ext_langs.record_path(path.as_str(), language.as_str());
            }

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

        // 2) Persisted type_info — restore TypeIds from the raw index columns.
        //    The arena was restored from its snapshot before this pass so raw
        //    indices map to the same interned types they did when persisted.
        if let Ok(mut ti_stmt) = conn.prepare(
            "SELECT s.qualified_name, s.name, \
                    t.generic_params, t.symbol_id, t.field_type_id, t.return_type_id \
             FROM symbol_type_info t JOIN symbols s ON s.id = t.symbol_id",
        ) {
            if let Ok(ti_rows) = ti_stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                ))
            }) {
                // Map a persisted raw TypeId index back to a TypeId — valid
                // because the arena was restored from the snapshot before this pass.
                let fid_from_raw = |raw: Option<i64>| -> Option<TypeId> {
                    raw.and_then(|n| u32::try_from(n).ok())
                        .and_then(std::num::NonZeroU32::new)
                        .map(TypeId)
                };
                for (
                    qname,
                    name,
                    generic_params_j,
                    symbol_id,
                    field_type_id_raw,
                    return_type_id_raw,
                ) in ti_rows.flatten()
                {
                    // A changed symbol's type_info is the fresh parse's, not the
                    // last index's persisted (possibly stale) row.
                    if parsed_qnames.contains(&qname) {
                        continue;
                    }
                    let param_names = parse_json_string_array(generic_params_j.as_deref());
                    // Re-intern each param name as a GenericParamData with owner=0,
                    // bound=None (the fallback load path; the snapshot carries the
                    // precise data for symbols whose TypeIds survived the arena reload).
                    let interned_ids: Vec<GenericParamId> = param_names
                        .iter()
                        .map(|n| {
                            self.arena.intern_generic(GenericParamData {
                                name: n.clone(),
                                owner_symbol_index: 0,
                                bound: None,
                            })
                        })
                        .collect();

                    let ti = self.type_info.entry(qname.clone()).or_default();
                    if ti.field_type_id.is_none() {
                        ti.field_type_id = fid_from_raw(field_type_id_raw);
                    }
                    if ti.return_type_id.is_none() {
                        ti.return_type_id = fid_from_raw(return_type_id_raw);
                    }
                    if ti.generic_param_ids.is_empty() && !interned_ids.is_empty() {
                        ti.generic_param_ids = interned_ids.clone();
                    }
                    // Rebuild the simple-name generic_param_ids key (the full pass
                    // stores generic params under both the qname and the bare name).
                    if !interned_ids.is_empty() && name != qname {
                        let sti = self.type_info.entry(name).or_default();
                        if sti.generic_param_ids.is_empty() {
                            sti.generic_param_ids = interned_ids.clone();
                        }
                    }
                    // Id-keyed slot — the persisted row is per-symbol-id, so this
                    // restores the overload-accurate return the resolver reads by
                    // id (`return_type_id_of`) on an incremental reload, with no
                    // qname-collapse.
                    let tid = self.type_info_by_id.entry(symbol_id).or_default();
                    if tid.field_type_id.is_none() {
                        tid.field_type_id = fid_from_raw(field_type_id_raw);
                    }
                    if tid.return_type_id.is_none() {
                        tid.return_type_id = fid_from_raw(return_type_id_raw);
                    }
                    if tid.generic_param_ids.is_empty() && !interned_ids.is_empty() {
                        tid.generic_param_ids = interned_ids;
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
    candidates: Vec<(String, String, Option<TypeId>)>,
) -> Vec<(String, String, Option<TypeId>)> {
    // qname → Some(candidate) on a single agreed type, None on a conflict.
    let mut by_fn: HashMap<String, Option<(String, Option<TypeId>)>> = HashMap::new();
    for (qname, ty, ty_id) in candidates {
        match by_fn.get(&qname) {
            None => {
                by_fn.insert(qname, Some((ty, ty_id)));
            }
            Some(Some((prev_ty, _))) if *prev_ty != ty => {
                by_fn.insert(qname, None);
            }
            _ => {}
        }
    }
    by_fn
        .into_iter()
        .filter_map(|(qname, agreed)| agreed.map(|(ty, ty_id)| (qname, ty, ty_id)))
        .collect()
}

/// Serialize a string vec to a JSON array, or `None` (SQL NULL) when empty.
pub(super) fn json_string_array(items: &[String]) -> Option<String> {
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
    candidates: Vec<(String, String, Option<TypeId>)>,
) -> Vec<(String, String, Option<TypeId>)> {
    agree_inferred_returns(candidates)
}

#[cfg(test)]
#[path = "compilation_tests.rs"]
mod tests;
