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
    parse_return_type_positional, parse_type_head_and_args, resolve_type_name_in_scope, Symbol,
    SymbolLookup, SymbolSet, TypeInfo,
};
use crate::indexer::project_context::ProjectContext;
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::types::{AliasTarget, EdgeKind, ParsedFile, SymbolKind};

// ---------------------------------------------------------------------------
// is_type_like — local mirror of the engine-util predicate
// ---------------------------------------------------------------------------

fn is_type_like(kind: &str) -> bool {
    matches!(
        kind,
        "class"
            | "struct"
            | "interface"
            | "enum"
            | "type_alias"
            | "namespace"
            | "record"
            | "trait"
            | "protocol"
            | "object"
            | "mixin"
            | "extension"
    )
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
    /// Re-export map: file_path → [(original_name, source_module)].
    /// Populated from `pf.refs` where `is_reexport` is true.
    reexport_map: FxHashMap<String, Vec<(String, String)>>,
    /// Direct-parent inheritance: child_qname → parent_qname (head, generics stripped).
    inherits: FxHashMap<String, String>,
    /// Direct-parent inheritance keyed by SYMBOL ID: child symbol id → parent
    /// symbol id — the id-keyed counterpart of `inherits`. Derived at the end of
    /// `ingest` / `ingest_from_db` by resolving each `inherits` child qname to its
    /// id and each parent head to a SPECIFIC parent symbol (a same-package
    /// candidate winning over a same-named type in another package). Lets the
    /// chain walker climb supertypes by identity, so an `extends Base` where the
    /// `Base` qname is duplicated across packages binds inherited members from the
    /// child's actual base, not the first-wins qname collision.
    inherits_by_id: FxHashMap<i64, i64>,
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
    /// against: the workspace-package declared-name → id map.
    fn snapshot_project_context(&mut self, ctx: &ProjectContext) {
        self.workspace_pkg_by_declared_name = ctx
            .workspace_pkg_by_declared_name
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
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
            reexport_map: FxHashMap::default(),
            inherits: FxHashMap::default(),
            inherits_by_id: FxHashMap::default(),
            enclosing_type: FxHashMap::default(),
            enclosing_namespace: FxHashMap::default(),
            alias_target: FxHashMap::default(),
            workspace_pkg_by_declared_name: FxHashMap::default(),
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
                let parent_key: String =
                    match sym.parent_index.and_then(|p| pf.symbols.get(p)) {
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
                // Strip generic args so the inherits map keys on the bare head.
                let parent_head = r
                    .target_name
                    .split('<')
                    .next()
                    .unwrap_or(&r.target_name)
                    .to_string();
                self.inherits
                    .entry(child_sym.qualified_name.clone())
                    .or_insert(parent_head);
            }

            // Pass 4 — re-export map from refs where `is_reexport` is true.
            for r in &pf.refs {
                if !r.is_reexport {
                    continue;
                }
                if let Some(module) = &r.module {
                    self.reexport_map
                        .entry(pf.path.clone())
                        .or_default()
                        .push((r.target_name.clone(), module.clone()));
                }
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
        for pf in parsed {
            self.derive_type_info_from_refs(pf);
        }

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

        // Id-keyed member index, derived from the now-complete `members_by_parent`
        // + `by_qname`. Rebuilt in full (not incrementally) because `ingest` runs
        // more than once — internal files, then the materialized externals — and
        // each run must reflect the cumulative member set. Stores child ids;
        // `members_of_id` resolves them through `by_id`.
        let mut members_by_id: FxHashMap<i64, Vec<i64>> = FxHashMap::default();
        for (parent_qname, members) in &self.members_by_parent {
            if let Some(parent) = self.by_qname.get(parent_qname) {
                members_by_id
                    .entry(parent.id)
                    .or_default()
                    .extend(members.iter().map(|m| m.id));
            }
        }
        self.members_by_id = members_by_id;

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
        let mut inherits_by_id: FxHashMap<i64, i64> = FxHashMap::default();
        for (child_qname, parent_head) in &self.inherits {
            let Some(child) = self.by_qname.get(child_qname) else {
                continue;
            };
            if let Some(parent_id) = self.resolve_parent_id(parent_head, child.package_id) {
                inherits_by_id.insert(child.id, parent_id);
            }
        }
        self.inherits_by_id = inherits_by_id;
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
    fn derive_type_info_from_refs(&mut self, pf: &ParsedFile) {
        // Collect TypeRef refs (excluding import bindings) per symbol index.
        let mut type_refs_by_sym: Vec<Vec<&str>> = vec![Vec::new(); pf.symbols.len()];
        for r in &pf.refs {
            if r.kind != EdgeKind::TypeRef || r.is_import_binding {
                continue;
            }
            if r.source_symbol_index < type_refs_by_sym.len() {
                type_refs_by_sym[r.source_symbol_index].push(r.target_name.as_str());
            }
        }

        for (sym_idx, sym) in pf.symbols.iter().enumerate() {
            let type_refs = &type_refs_by_sym[sym_idx];
            match sym.kind {
                SymbolKind::Property
                | SymbolKind::Field
                | SymbolKind::Variable
                | SymbolKind::Parameter => {
                    let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                    if ti.field_type.is_none() {
                        if let Some(&first) = type_refs.first() {
                            let resolved = resolve_type_name_in_scope(
                                first,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            let arg_strs: Vec<String> = if type_refs.len() > 1 {
                                type_refs[1..].iter().map(|s| s.to_string()).collect()
                            } else {
                                Vec::new()
                            };
                            ti.field_type_id =
                                Some(intern_head_and_args(&self.arena, &resolved, &arg_strs));
                            ti.field_type = Some(resolved);
                            ti.type_args = arg_strs;
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
                                ti.field_type_id = Some(self.arena.class(&resolved));
                                ti.field_type = Some(resolved);
                            }
                        }
                    }
                }
                SymbolKind::TypeAlias => {
                    if let Some(&first) = type_refs.first() {
                        let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                        if ti.field_type.is_none() {
                            ti.field_type_id = Some(self.arena.intern_type_str(first));
                            ti.field_type = Some(first.to_string());
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

                    let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                    if ti.return_type.is_none() {
                        if let Some((head, args)) = sig_generic {
                            let resolved = resolve_type_name_in_scope(
                                &head,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            ti.return_type_id =
                                Some(intern_head_and_args(&self.arena, &resolved, &args));
                            ti.return_type = Some(resolved);
                            ti.return_type_args = args;
                        } else if let Some(&last) = type_refs.last() {
                            let resolved = resolve_type_name_in_scope(
                                last,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            ti.return_type_id = Some(self.arena.intern_type_str(&resolved));
                            ti.return_type = Some(resolved);
                        } else if let Some(rt) = &sig_rt {
                            let resolved = resolve_type_name_in_scope(
                                rt,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            ti.return_type_id = Some(self.arena.intern_type_str(&resolved));
                            ti.return_type = Some(resolved);
                        }
                    }
                }
                SymbolKind::Class => {
                    // Constructor call yields the class itself.
                    let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                    if ti.return_type.is_none() {
                        ti.return_type_id = Some(self.arena.class(&sym.qualified_name));
                        ti.return_type = Some(sym.qualified_name.clone());
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
                            let (params, bounds): (Vec<String>, Vec<Option<String>>) =
                                gparams.into_iter().unzip();
                            // Key on both simple name and qname so callers
                            // using either form get the params.
                            for key in [&sym.name, &sym.qualified_name] {
                                let ti =
                                    self.type_info.entry(key.clone()).or_default();
                                if ti.generic_params.is_empty() {
                                    ti.generic_params = params.clone();
                                    ti.generic_param_bounds = bounds.clone();
                                }
                            }
                            break;
                        }
                    }
                }
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
            ti.return_type_args = type_args;
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

    fn field_type_args(&self, property_qname: &str) -> Option<&[String]> {
        self.type_info
            .get(property_qname)
            .filter(|ti| !ti.type_args.is_empty())
            .map(|ti| ti.type_args.as_slice())
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
        self.inherits.get(class_qname).map(|s| s.as_str())
    }

    fn parent_class_id(&self, child_id: i64) -> Option<i64> {
        self.inherits_by_id.get(&child_id).copied()
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
                 (symbol_id, field_type, return_type, type_args, return_type_args, generic_params) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for (qname, ti) in &self.type_info {
                let Some(sym) = self.by_qname.get(qname) else {
                    continue;
                };
                if ti.field_type.is_none()
                    && ti.return_type.is_none()
                    && ti.type_args.is_empty()
                    && ti.return_type_args.is_empty()
                    && ti.generic_params.is_empty()
                {
                    continue;
                }
                stmt.execute(rusqlite::params![
                    sym.id,
                    ti.field_type.as_deref(),
                    ti.return_type.as_deref(),
                    json_string_array(&ti.type_args),
                    json_string_array(&ti.return_type_args),
                    json_string_array(&ti.generic_params),
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
                    s.visibility, f.package_id, s.signature \
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
            ))
        }) else {
            return id_map;
        };
        for (id, name, qname, kind, path, scope_path, visibility, package_id, signature) in
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
        }

        // 2) Persisted type_info, re-interned into this build's fresh arena.
        if let Ok(mut ti_stmt) = conn.prepare(
            "SELECT s.qualified_name, s.name, t.field_type, t.return_type, \
                    t.type_args, t.return_type_args, t.generic_params \
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
                    r.get::<_, Option<String>>(6)?,
                ))
            }) {
                for (qname, name, field_type, return_type, type_args_j, return_type_args_j, generic_params_j) in
                    ti_rows.flatten()
                {
                    // A changed symbol's type_info is the fresh parse's, not the
                    // last index's persisted (possibly stale) row.
                    if parsed_qnames.contains(&qname) {
                        continue;
                    }
                    let type_args = parse_json_string_array(type_args_j.as_deref());
                    let return_type_args = parse_json_string_array(return_type_args_j.as_deref());
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
                    if ti.return_type_args.is_empty() {
                        ti.return_type_args = return_type_args;
                    }
                    if ti.generic_params.is_empty() && !generic_params.is_empty() {
                        ti.generic_params = generic_params.clone();
                    }
                    // Rebuild the simple-name generic_params key (the full pass
                    // stores generic params under both the qname and the bare name).
                    if !generic_params.is_empty() && name != qname {
                        let sti = self.type_info.entry(name).or_default();
                        if sti.generic_params.is_empty() {
                            sti.generic_params = generic_params;
                        }
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
                    let head = parent.split('<').next().unwrap_or(&parent).to_string();
                    self.inherits.entry(child).or_insert(head);
                }
            }
        }

        // 4) Rebuild the id-keyed member index over the now-complete maps.
        let mut members_by_id: FxHashMap<i64, Vec<i64>> = FxHashMap::default();
        for (parent_qname, members) in &self.members_by_parent {
            if let Some(parent) = self.by_qname.get(parent_qname) {
                members_by_id
                    .entry(parent.id)
                    .or_default()
                    .extend(members.iter().map(|m| m.id));
            }
        }
        self.members_by_id = members_by_id;

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
