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
use crate::type_checker::core::types::{TypeArena, TypeId};
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
                            ti.field_type = Some(resolved);
                            if type_refs.len() > 1 {
                                ti.type_args = type_refs[1..]
                                    .iter()
                                    .map(|s| s.to_string())
                                    .collect();
                            }
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
                                ti.field_type = Some(resolved);
                            }
                        }
                    }
                }
                SymbolKind::TypeAlias => {
                    if let Some(&first) = type_refs.first() {
                        let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                        if ti.field_type.is_none() {
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
                            ti.return_type = Some(resolved);
                            ti.return_type_args = args;
                        } else if let Some(&last) = type_refs.last() {
                            let resolved = resolve_type_name_in_scope(
                                last,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            ti.return_type = Some(resolved);
                        } else if let Some(rt) = &sig_rt {
                            let resolved = resolve_type_name_in_scope(
                                rt,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            ti.return_type = Some(resolved);
                        }
                    }
                }
                SymbolKind::Class => {
                    // Constructor call yields the class itself.
                    let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                    if ti.return_type.is_none() {
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

/// Test-only re-export of the type-like predicate so `tree_tests.rs` can
/// assert that `types_by_name` only surfaces type-like symbols without
/// duplicating the match arms.
#[cfg(test)]
pub(super) fn is_type_like_for_test(kind: &str) -> bool {
    is_type_like(kind)
}

#[cfg(test)]
#[path = "compilation_tests.rs"]
mod tests;
