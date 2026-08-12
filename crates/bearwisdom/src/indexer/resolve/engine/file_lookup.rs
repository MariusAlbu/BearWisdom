// =============================================================================
// engine/file_lookup — per-file SymbolLookup overlay
//
// Wraps the Compilation with the two per-file concerns every rule and the
// chain walker inherit through `ctx.lookup`: cross-language external
// visibility on the candidate-set delegates, and the forward-inference cache
// for local binding types.
// =============================================================================

use std::cell::RefCell;

use rustc_hash::FxHashMap;

use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::engine::contract::{FlowCacheLookup, Symbol, SymbolLookup, SymbolSet};
use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::types::AliasTargetIds;

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
pub(super) struct FileLookup<'a> {
    tree: &'a Compilation,
    /// Candidate-language codes whose EXTERNAL declarations this file's
    /// language may bind by name (`Compilation::ext_lang_allowed`). `None`
    /// disables the check. Applied by the candidate-set delegates
    /// (`by_name` / `types_by_name` / `ambient_symbols` /
    /// `by_qualified_name` / `all_by_qualified_name`) so every rule and the
    /// chain walker inherit one visibility — a dotted target from source
    /// text is not cross-language evidence, so qualified probes filter the
    /// same way bare-name ones do.
    allowed_ext_langs: Option<&'a rustc_hash::FxHashSet<u16>>,
    locals: RefCell<FxHashMap<String, String>>,
    locals_id: RefCell<FxHashMap<String, TypeId>>,
    /// First-uncaptured-type cause recorded when a local binding's forward-
    /// inference seed failed — e.g. `const x = f()` where `f` resolved but
    /// its own return type was never captured. Consulted by the chain
    /// walker's root step when `locals`/`locals_id` carry no entry for the
    /// name, so a later `x.method()` blames `f`, not `x`.
    root_cause_hints: RefCell<FxHashMap<String, crate::indexer::resolve::engine::cause::Cause>>,
    /// Name → declaration qname for a binding whose field/return type was
    /// never captured (a destructured `$Ret`-synthesized member). Read only by
    /// `LocalFlowHeadRule`'s bare-name-call resolution — kept separate from
    /// `locals`/`locals_id` so it never re-roots a chain that continues past
    /// this binding onto the member-less `$Ret` leaf.
    local_callable_heads: RefCell<FxHashMap<String, String>>,
}

impl<'a> FileLookup<'a> {
    pub(super) fn new(tree: &'a Compilation, language: &str) -> Self {
        Self {
            tree,
            allowed_ext_langs: tree.ext_lang_allowed(language),
            locals: RefCell::new(FxHashMap::default()),
            locals_id: RefCell::new(FxHashMap::default()),
            root_cause_hints: RefCell::new(FxHashMap::default()),
            local_callable_heads: RefCell::new(FxHashMap::default()),
        }
    }
}

impl<'a> SymbolLookup for FileLookup<'a> {
    // -- Structural delegation: 22 methods forwarded directly to the tree. ----

    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        self.tree
            .filter_ext_langs(self.tree.by_name(name), self.allowed_ext_langs)
    }

    // Qualified lookups are visibility-filtered too: a dotted target from
    // source text (`Option.map`) is not import evidence, and a qname collision
    // across languages otherwise binds a foreign external. The canonical
    // first-wins pick is kept whenever the receiver may bind it; only a
    // blocked pick falls back to the first allowed same-qname candidate.
    fn by_qualified_name(&self, qname: &str) -> Option<&Symbol> {
        let sym = self.tree.by_qualified_name(qname)?;
        if self
            .tree
            .filter_ext_langs(SymbolSet::Owned(vec![sym]), self.allowed_ext_langs)
            .into_iter()
            .next()
            .is_some()
        {
            return Some(sym);
        }
        self.tree
            .filter_ext_langs(self.tree.all_by_qualified_name(qname), self.allowed_ext_langs)
            .into_iter()
            .next()
    }

    fn all_by_qualified_name(&self, qname: &str) -> SymbolSet<'_> {
        self.tree
            .filter_ext_langs(self.tree.all_by_qualified_name(qname), self.allowed_ext_langs)
    }

    fn members_of(&self, parent_qname: &str) -> SymbolSet<'_> {
        self.tree.members_of(parent_qname)
    }

    fn members_of_id(&self, parent_id: i64) -> SymbolSet<'_> {
        self.tree.members_of_id(parent_id)
    }

    fn types_by_name(&self, name: &str) -> SymbolSet<'_> {
        self.tree
            .filter_ext_langs(self.tree.types_by_name(name), self.allowed_ext_langs)
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
        self.tree
            .filter_ext_langs(self.tree.ambient_symbols(name), self.allowed_ext_langs)
    }

    fn field_type_name(&self, property_qname: &str) -> Option<&str> {
        self.tree.field_type_name(property_qname)
    }

    fn return_type_name(&self, method_qname: &str) -> Option<&str> {
        self.tree.return_type_name(method_qname)
    }

    fn generic_params(&self, type_name: &str) -> Option<Vec<String>> {
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

    fn field_type_id_of(&self, symbol_id: i64) -> Option<TypeId> {
        self.tree.field_type_id_of(symbol_id)
    }

    fn generic_params_of(&self, symbol_id: i64) -> Option<Vec<String>> {
        self.tree.generic_params_of(symbol_id)
    }

    fn generic_param_defaults_of(&self, symbol_id: i64) -> Option<Vec<Option<String>>> {
        self.tree.generic_param_defaults_of(symbol_id)
    }

    fn symbol_by_id(&self, id: i64) -> Option<&Symbol> {
        self.tree.symbol_by_id(id)
    }

    fn type_arena(&self) -> Option<&TypeArena> {
        self.tree.type_arena()
    }

    fn alias_target(&self, name: &str) -> Option<&AliasTargetIds> {
        self.tree.alias_target(name)
    }

    fn alias_target_by_id(&self, id: i64) -> Option<&AliasTargetIds> {
        self.tree.alias_target_by_id(id)
    }

    fn reexports_from(&self, file_path: &str) -> &[(String, String)] {
        self.tree.reexports_from(file_path)
    }

    fn resolve_module_from(&self, source_file: &str, spec: &str) -> Option<&str> {
        self.tree.resolve_module_from(source_file, spec)
    }

    fn resolve_module_via_language_resolver(
        &self,
        language: &str,
        source_file: &str,
        spec: &str,
    ) -> Option<String> {
        self.tree.resolve_module_via_language_resolver(language, source_file, spec)
    }

    fn in_module_from(&self, source_file: &str, spec: &str) -> SymbolSet<'_> {
        self.tree.in_module_from(source_file, spec)
    }

    fn resolve_external_reexport(&self, target: &str, prefix: &str, module: &str) -> Option<i64> {
        self.tree.resolve_external_reexport(target, prefix, module)
    }

    fn reexport_alias_target(&self, qname: &str) -> Option<&Symbol> {
        self.tree.reexport_alias_target(qname)
    }

    fn selector_qname(&self, raw_selector: &str) -> Option<&str> {
        self.tree.selector_qname(raw_selector)
    }

    fn is_external_name(&self, name: &str, language: &str) -> bool {
        self.tree.is_external_name(name, language)
    }

    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        self.tree.parent_class_qname(class_qname)
    }
    fn parent_class_qnames(&self, class_qname: &str) -> &[String] {
        self.tree.parent_class_qnames(class_qname)
    }

    fn parent_class_id(&self, child_id: i64) -> Option<i64> {
        self.tree.parent_class_id(child_id)
    }

    fn parent_class_ids(&self, child_id: i64) -> Vec<i64> {
        self.tree.parent_class_ids(child_id)
    }

    fn parent_class_args(&self, child_head: &str, parent_head: &str) -> &[String] {
        self.tree.parent_class_args(child_head, parent_head)
    }

    fn parent_class_arg_ids(&self, child_head: &str, parent_head: &str) -> &[TypeId] {
        self.tree.parent_class_arg_ids(child_head, parent_head)
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

    fn resolve_path_alias(&self, package_id: Option<i64>, specifier: &str) -> Option<String> {
        self.tree.resolve_path_alias(package_id, specifier)
    }

    fn implicit_wildcard_namespaces(&self, package_id: Option<i64>) -> &[String] {
        self.tree.implicit_wildcard_namespaces(package_id)
    }

    fn dep_rename(&self, consumer_pkg: Option<i64>, alias: &str) -> Option<&str> {
        self.tree.dep_rename(consumer_pkg, alias)
    }

    fn package_id_for_file(&self, file_path: &str) -> Option<i64> {
        self.tree.package_id_for_file(file_path)
    }

}

// Flow cache: methods implemented over `locals` and `locals_id`.
impl<'a> FlowCacheLookup for FileLookup<'a> {
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
    ///
    /// Does NOT evict `root_cause_hints` — that cache is consulted only as
    /// `resolve_root`'s last resort, strictly after `local_type_id`/`local_type`
    /// both miss, so a stale hint left behind by an earlier failed seed for
    /// this name is never read once this record succeeds.
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

    fn record_root_cause_hint(&self, name: String, cause: crate::indexer::resolve::engine::cause::Cause) {
        self.root_cause_hints.borrow_mut().insert(name, cause);
    }

    fn local_callable_head(&self, name: &str) -> Option<String> {
        self.local_callable_heads.borrow().get(name).cloned()
    }

    fn record_local_callable_head(&self, name: String, qname: String) {
        self.local_callable_heads.borrow_mut().insert(name, qname);
    }

    fn root_cause_hint(&self, name: &str) -> Option<crate::indexer::resolve::engine::cause::Cause> {
        self.root_cause_hints.borrow().get(name).copied()
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
        self.root_cause_hints.borrow_mut().clear();
        self.local_callable_heads.borrow_mut().clear();
    }
}
