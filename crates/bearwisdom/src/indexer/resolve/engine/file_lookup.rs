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
/// Syntax-migrated files use LexicalCache: ScopeId/BindingId-keyed typed facts.
/// The maps below are the legacy path for languages/files without lexical
/// metadata; they are never consulted when the scoped cache is installed.
/// CFG joins and identity-bearing contextual lambda writes remain separate work.
pub(super) struct FileLookup<'a> {
    tree: &'a Compilation,
    program: Option<super::program_view::Lookup<'a>>,
    module_site: super::module_graph::ModuleSite,
    lexical: Option<super::lexical_cache::LexicalCache<'a>>,
    namespace_roots: FxHashMap<u32, super::contract::flow_cache::LocalReference>,
    namespace_selectors: FxHashMap<u32, super::contract::flow_cache::LocalReference>,
    method_calls: FxHashMap<u32, i64>,
    borrow_sites:
        FxHashMap<crate::types::SourceSpan, (i64, crate::type_checker::core::types::Mutability)>,
    method_names: FxHashMap<u32, super::member_index::MemberNameId>,
    private_members: FxHashMap<u32, Option<i64>>,
    trait_file: Option<&'a super::trait_graph::File>,
    call_arguments: Option<&'a crate::indexer::namespaces::arguments::Table>,
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
    fn structural(&self) -> &dyn SymbolLookup {
        self.program
            .as_ref()
            .map(|p| p as &dyn SymbolLookup)
            .unwrap_or(self.tree)
    }
    fn root_candidates<'s>(&self, symbols: SymbolSet<'s>) -> SymbolSet<'s> {
        if self.program.is_some() {
            return SymbolSet::empty();
        }
        self.tree
            .root_candidates(self.tree.filter_ext_langs(symbols, self.allowed_ext_langs))
    }
    pub(super) fn new(tree: &'a Compilation, language: &str) -> Self {
        Self {
            tree,
            program: None,
            module_site: Default::default(),
            lexical: None,
            namespace_roots: FxHashMap::default(),
            namespace_selectors: FxHashMap::default(),
            method_calls: FxHashMap::default(),
            borrow_sites: FxHashMap::default(),
            method_names: FxHashMap::default(),
            private_members: FxHashMap::default(),
            trait_file: None,
            call_arguments: None,
            allowed_ext_langs: tree.ext_lang_allowed(language),
            locals: RefCell::new(FxHashMap::default()),
            locals_id: RefCell::new(FxHashMap::default()),
            root_cause_hints: RefCell::new(FxHashMap::default()),
            local_callable_heads: RefCell::new(FxHashMap::default()),
        }
    }

    pub(super) fn for_file(
        tree: &'a Compilation,
        pf: &'a crate::types::ParsedFile,
        ids: &crate::indexer::symbol_ids::SymbolIds,
    ) -> Self {
        let mut lookup = Self::new(tree, &pf.language);
        lookup.program = tree.source_program_lookup(pf);
        if let Some(program) = &lookup.program {
            if let Some(graph) = &pf.flow.lexical {
                lookup.method_names = graph
                    .globals
                    .iter()
                    .flat_map(|g| &g.selectors)
                    .filter_map(|(&byte, &name)| {
                        program.source_member_id(name).map(|name| (byte, name))
                    })
                    .collect();
                lookup.private_members = graph
                    .globals
                    .iter()
                    .flat_map(|g| &g.private_selectors)
                    .map(|(&byte, slot)| (byte, slot.and_then(|slot| ids.row_id(&pf.path, slot))))
                    .collect();
            }
        }
        lookup.module_site = tree.module_site(&pf.path);
        lookup.trait_file = tree.trait_file(&pf.path);
        if let Some(data) = &pf.flow.namespaces {
            lookup.call_arguments = Some(&data.call_arguments);
            // Ingestion bridge only: the reference loop carries selector IDs,
            // and never interns or compares method spellings during selection.
            lookup.method_names = pf
                .refs
                .iter()
                .filter_map(|r| r.chain.as_ref())
                .flat_map(|c| &c.segments)
                .filter(|s| {
                    data.method_calls.contains_key(&s.byte_offset)
                        || data.traits.qualified_calls.contains_key(&s.byte_offset)
                })
                .filter_map(|s| {
                    tree.member_index()?
                        .name(&s.name)
                        .map(|name| (s.byte_offset, name))
                })
                .collect();
            lookup.method_calls = data
                .method_calls
                .iter()
                .filter_map(|(&byte, &slot)| ids.row_id(&pf.path, slot).map(|id| (byte, id)))
                .collect();
            lookup.borrow_sites = data
                .borrow_sites
                .iter()
                .filter_map(|(&span, &(slot, mutable))| {
                    ids.row_id(&pf.path, slot).map(|id| (span, (id, mutable)))
                })
                .collect();
            lookup.namespace_roots = data
                .roots
                .iter()
                .filter_map(|(&byte, &usage)| {
                    tree.namespace_use(&pf.path, usage)
                        .map(|value| (byte, value))
                })
                .collect();
            lookup.namespace_selectors = data
                .selectors
                .iter()
                .filter_map(|(&byte, &usage)| {
                    tree.namespace_use(&pf.path, usage)
                        .map(|value| (byte, value))
                })
                .collect();
            for call in data.traits.qualified_calls.values() {
                // Qualified syntax is an authoritative root even if its explicit
                // trait/Self recipe cannot yet be materialized.
                lookup.namespace_roots.insert(
                    call.root.start,
                    super::contract::flow_cache::LocalReference {
                        declaration: None,
                        kind: crate::types::SymbolKind::Variable,
                        value_type: None,
                        type_args: Vec::new(),
                        callable: None,
                    },
                );
            }
        }
        lookup.lexical = pf.flow.lexical.as_ref().and_then(|bindings| {
            tree.type_arena()
                .map(|arena| super::lexical_cache::LexicalCache::new(bindings, arena))
        });
        if let Some(cache) = &mut lookup.lexical {
            cache.install_declarations(&pf.path, ids);
            let selected: &dyn SymbolLookup = lookup
                .program
                .as_ref()
                .map(|p| p as &dyn SymbolLookup)
                .unwrap_or(tree);
            cache.install_types(&pf.path, ids, tree, selected);
            cache.install_initial_values(selected);
            cache.install_imports(&pf.path, selected);
            for (&byte, value) in lookup
                .namespace_roots
                .iter_mut()
                .chain(&mut lookup.namespace_selectors)
            {
                if let Some(arguments) = cache.member_type_arguments(byte) {
                    value.type_args = arguments.to_vec();
                }
            }
        }
        lookup
    }
}

impl<'a> SymbolLookup for FileLookup<'a> {
    fn bound_import_namespace(
        &self,
        file: &str,
        binding: crate::indexer::lexical::BindingId,
    ) -> bool {
        self.structural().bound_import_namespace(file, binding)
    }
    fn bound_import_overloads(
        &self,
        file: &str,
        binding: crate::indexer::lexical::BindingId,
    ) -> &[i64] {
        self.structural().bound_import_overloads(file, binding)
    }
    fn bound_import(
        &self,
        file: &str,
        binding: crate::indexer::lexical::BindingId,
        type_space: bool,
    ) -> Option<i64> {
        self.structural().bound_import(file, binding, type_space)
    }
    // -- Structural delegation: 22 methods forwarded directly to the tree. ----

    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        self.root_candidates(self.structural().by_name(name))
    }

    // Qualified lookups are visibility-filtered too: a dotted target from
    // source text (`Option.map`) is not import evidence, and a qname collision
    // across languages otherwise binds a foreign external. The canonical
    // first-wins pick is kept whenever the receiver may bind it; only a
    // blocked pick falls back to the first allowed same-qname candidate.
    fn by_qualified_name(&self, qname: &str) -> Option<&Symbol> {
        let sym = self.structural().by_qualified_name(qname)?;
        if self
            .root_candidates(SymbolSet::Owned(vec![sym]))
            .into_iter()
            .next()
            .is_some()
        {
            return Some(sym);
        }
        self.root_candidates(self.structural().all_by_qualified_name(qname))
            .into_iter()
            .next()
    }

    fn all_by_qualified_name(&self, qname: &str) -> SymbolSet<'_> {
        self.root_candidates(self.structural().all_by_qualified_name(qname))
    }

    fn members_of(&self, parent_qname: &str) -> SymbolSet<'_> {
        self.structural().members_of(parent_qname)
    }

    fn members_of_id(&self, parent_id: i64) -> SymbolSet<'_> {
        self.structural().members_of_id(parent_id)
    }

    fn member_index(&self) -> Option<&super::member_index::MemberIndex> {
        self.structural().member_index()
    }

    fn types_by_name(&self, name: &str) -> SymbolSet<'_> {
        self.root_candidates(self.structural().types_by_name(name))
    }

    fn in_namespace(&self, namespace: &str) -> Vec<&Symbol> {
        self.root_candidates(SymbolSet::Owned(self.tree.in_namespace(namespace)))
            .into_iter()
            .collect()
    }

    fn has_in_namespace(&self, namespace: &str) -> bool {
        self.tree.has_in_namespace(namespace)
    }

    fn in_file(&self, file_path: &str) -> SymbolSet<'_> {
        self.root_candidates(self.tree.in_file(file_path))
    }

    fn ambient_symbols(&self, name: &str) -> SymbolSet<'_> {
        self.root_candidates(self.structural().ambient_symbols(name))
    }

    fn field_type_name(&self, property_qname: &str) -> Option<&str> {
        self.structural().field_type_name(property_qname)
    }

    fn return_type_name(&self, method_qname: &str) -> Option<&str> {
        self.structural().return_type_name(method_qname)
    }

    fn generic_params(&self, type_name: &str) -> Option<Vec<String>> {
        self.structural().generic_params(type_name)
    }

    fn field_type_id(&self, property_qname: &str) -> Option<TypeId> {
        self.structural().field_type_id(property_qname)
    }

    fn return_type_id(&self, method_qname: &str) -> Option<TypeId> {
        self.structural().return_type_id(method_qname)
    }

    fn return_type_id_of(&self, symbol_id: i64) -> Option<TypeId> {
        self.structural().return_type_id_of(symbol_id)
    }
    fn generic_return_of(
        &self,
        id: i64,
    ) -> Option<&super::contract::generic_return::GenericReturn> {
        self.structural().generic_return_of(id)
    }

    fn field_type_id_of(&self, symbol_id: i64) -> Option<TypeId> {
        self.structural().field_type_id_of(symbol_id)
    }

    fn generic_params_of(&self, symbol_id: i64) -> Option<Vec<String>> {
        self.structural().generic_params_of(symbol_id)
    }

    fn canonical_type_info(&self, symbol_id: i64) -> Option<&super::contract::TypeInfo> {
        self.structural().canonical_type_info(symbol_id)
    }

    fn generic_param_defaults_of(&self, symbol_id: i64) -> Option<Vec<Option<String>>> {
        self.structural().generic_param_defaults_of(symbol_id)
    }

    fn symbol_by_id(&self, id: i64) -> Option<&Symbol> {
        self.structural().symbol_by_id(id)
    }

    fn type_arena(&self) -> Option<&TypeArena> {
        self.tree.type_arena()
    }

    fn alias_target(&self, name: &str) -> Option<&AliasTargetIds> {
        self.structural().alias_target(name)
    }

    fn alias_target_by_id(&self, id: i64) -> Option<&AliasTargetIds> {
        self.structural().alias_target_by_id(id)
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
        self.tree
            .resolve_module_via_language_resolver(language, source_file, spec)
    }

    fn in_module_from(&self, source_file: &str, spec: &str) -> SymbolSet<'_> {
        self.root_candidates(self.tree.in_module_from(source_file, spec))
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

    fn is_declared_dependency(&self, package_id: Option<i64>, spec: &str) -> bool {
        self.tree.is_declared_dependency(package_id, spec)
    }

    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        self.tree.parent_class_qname(class_qname)
    }
    fn parent_class_qnames(&self, class_qname: &str) -> &[String] {
        self.tree.parent_class_qnames(class_qname)
    }

    fn parent_class_id(&self, child_id: i64) -> Option<i64> {
        self.structural().parent_class_id(child_id)
    }

    fn parent_class_ids(&self, child_id: i64) -> Vec<i64> {
        self.structural().parent_class_ids(child_id)
    }

    fn parent_class_args(&self, child_head: &str, parent_head: &str) -> &[String] {
        self.tree.parent_class_args(child_head, parent_head)
    }

    fn parent_class_arg_ids(&self, child_head: &str, parent_head: &str) -> &[TypeId] {
        self.tree.parent_class_arg_ids(child_head, parent_head)
    }

    fn parent_class_arg_ids_of(&self, child_id: i64, parent_id: i64) -> &[TypeId] {
        self.structural()
            .parent_class_arg_ids_of(child_id, parent_id)
    }

    fn canonical_decl_id(&self, id: i64) -> i64 {
        self.structural().canonical_decl_id(id)
    }

    fn enclosing_type_qname(&self, source_qname: &str) -> Option<&str> {
        self.tree.enclosing_type_qname(source_qname)
    }

    fn enclosing_type_id_of(&self, source_symbol_id: i64) -> Option<i64> {
        self.structural().enclosing_type_id_of(source_symbol_id)
    }

    fn enclosing_namespace_qname(&self, source_qname: &str) -> Option<&str> {
        self.tree.enclosing_namespace_qname(source_qname)
    }

    fn symbols_in_package(&self, package_id: i64) -> SymbolSet<'_> {
        self.root_candidates(self.tree.symbols_in_package(package_id))
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

#[path = "file_lookup_flow.rs"]
mod flow_cache;
