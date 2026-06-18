// =============================================================================
// indexer/resolve/engine/testkit — shared synthetic fixtures for rule tests
//
// A lean `SymbolLookup` double plus builders for the surrounding context, so
// each rule's sibling `_tests.rs` drives the rule without spinning up a DB. Only
// the lookup methods the rules read are wired; the rest take their trait
// defaults. Test-only.
// =============================================================================

use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::indexer::resolve::engine::contract::{
    FileContext, ImportEntry, RefContext, Symbol, SymbolLookup, SymbolSet,
};
use crate::type_checker::core::types::TypeArena;
use crate::types::{AliasTarget, EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};

/// Synthetic symbol index. Register symbols with `with`, members with
/// `with_member`; everything else is empty.
pub(crate) struct Lookup {
    empty: Vec<Symbol>,
    empty_pairs: Vec<(String, String)>,
    by_name: FxHashMap<String, Vec<Symbol>>,
    by_qname: FxHashMap<String, Symbol>,
    by_qname_all: FxHashMap<String, Vec<Symbol>>,
    members: FxHashMap<String, Vec<Symbol>>,
    /// Id-keyed members: parent symbol id → member symbols. The id-keyed
    /// counterpart of `members`, exercising the chain walker's identity path.
    members_by_id: FxHashMap<i64, Vec<Symbol>>,
    generics: FxHashMap<String, Vec<String>>,
    field_types: FxHashMap<String, String>,
    return_types: FxHashMap<String, String>,
    parents: FxHashMap<String, String>,
    /// Id-keyed inherits: child symbol id → ALL parent symbol ids. The id-keyed
    /// counterpart of `parents`; a child may have several supertypes.
    parents_by_id: FxHashMap<i64, Vec<i64>>,
    local_types: FxHashMap<String, String>,
    enclosing: FxHashMap<String, String>,
    aliases: FxHashMap<String, AliasTarget>,
    ambient: FxHashMap<String, Vec<Symbol>>,
    /// Package id → symbols, the id-keyed counterpart of the real `by_package`
    /// map. Backs `symbols_in_package` for workspace-scoped rule tests.
    by_package: FxHashMap<i64, Vec<Symbol>>,
    /// Declared workspace-package specifier → package id, backing
    /// `workspace_package_id` / `is_workspace_declared_name`.
    workspace_pkgs: FxHashMap<String, i64>,
    /// Workspace arena the chain walker interns roots / yields into. Mirrors the
    /// real `Compilation`, which owns one; the chain walk declines without it.
    arena: TypeArena,
}

impl Lookup {
    pub(crate) fn new() -> Self {
        Self {
            empty: Vec::new(),
            empty_pairs: Vec::new(),
            by_name: Default::default(),
            by_qname: Default::default(),
            by_qname_all: Default::default(),
            members: Default::default(),
            members_by_id: Default::default(),
            generics: Default::default(),
            field_types: Default::default(),
            return_types: Default::default(),
            parents: Default::default(),
            parents_by_id: Default::default(),
            local_types: Default::default(),
            enclosing: Default::default(),
            aliases: Default::default(),
            ambient: Default::default(),
            by_package: Default::default(),
            workspace_pkgs: Default::default(),
            arena: TypeArena::new(),
        }
    }

    /// Register a symbol under both its simple name and its qualified name.
    pub(crate) fn with(mut self, sym: Symbol) -> Self {
        self.by_name
            .entry(sym.name.clone())
            .or_default()
            .push(sym.clone());
        self.by_qname_all
            .entry(sym.qualified_name.clone())
            .or_default()
            .push(sym.clone());
        self.by_qname.insert(sym.qualified_name.clone(), sym);
        self
    }

    /// Register a member symbol under `parent_qname`.
    pub(crate) fn with_member(mut self, parent_qname: &str, sym: Symbol) -> Self {
        self.members
            .entry(parent_qname.to_string())
            .or_default()
            .push(sym);
        self
    }

    /// Register a symbol that belongs to a workspace package. Indexes it under
    /// its simple/qualified name like `with`, AND records it under `package_id`
    /// (with `package_id` stamped on the row) so `symbols_in_package` and the
    /// import-scoped candidate pick can find it.
    pub(crate) fn with_in_package(mut self, package_id: i64, mut sym: Symbol) -> Self {
        sym.package_id = Some(package_id);
        self = self.with(sym.clone());
        self.by_package.entry(package_id).or_default().push(sym);
        self
    }

    /// Map a workspace-package specifier to its package id, backing
    /// `workspace_package_id` / `is_workspace_declared_name`.
    pub(crate) fn with_workspace_pkg(mut self, specifier: &str, package_id: i64) -> Self {
        self.workspace_pkgs.insert(specifier.to_string(), package_id);
        self
    }

    /// Register a member symbol under its parent's SYMBOL ID — the id-keyed
    /// counterpart of `with_member`, for tests that exercise the chain walker's
    /// identity path (two same-qname parents kept distinct by id).
    pub(crate) fn with_member_id(mut self, parent_id: i64, sym: Symbol) -> Self {
        self.members_by_id.entry(parent_id).or_default().push(sym);
        self
    }

    /// Register `child`'s direct parent by SYMBOL ID — the id-keyed counterpart
    /// of `with_parent`, driving the id-keyed supertype climb.
    pub(crate) fn with_parent_id(mut self, child_id: i64, parent_id: i64) -> Self {
        self.parents_by_id.entry(child_id).or_default().push(parent_id);
        self
    }

    /// Register generic parameter names for a type qname.
    pub(crate) fn with_generics(mut self, qname: &str, params: &[&str]) -> Self {
        self.generics.insert(
            qname.to_string(),
            params.iter().map(|s| s.to_string()).collect(),
        );
        self
    }

    /// Register the declared type of a field/property qname.
    pub(crate) fn with_field_type(mut self, qname: &str, ty: &str) -> Self {
        self.field_types.insert(qname.to_string(), ty.to_string());
        self
    }

    /// Register the return type of a method/function qname.
    pub(crate) fn with_return_type(mut self, qname: &str, ty: &str) -> Self {
        self.return_types.insert(qname.to_string(), ty.to_string());
        self
    }

    /// Register `child`'s direct parent type qname (an `extends`/`implements` link).
    pub(crate) fn with_parent(mut self, child_qname: &str, parent_qname: &str) -> Self {
        self.parents
            .insert(child_qname.to_string(), parent_qname.to_string());
        self
    }

    /// Register a local variable's forward-inferred type.
    pub(crate) fn with_local_type(mut self, name: &str, ty: &str) -> Self {
        self.local_types.insert(name.to_string(), ty.to_string());
        self
    }

    /// Register the enclosing type qname for a source symbol qname.
    pub(crate) fn with_enclosing(mut self, source_qname: &str, type_qname: &str) -> Self {
        self.enclosing
            .insert(source_qname.to_string(), type_qname.to_string());
        self
    }

    /// Register a type alias's target.
    pub(crate) fn with_alias(mut self, name: &str, target: AliasTarget) -> Self {
        self.aliases.insert(name.to_string(), target);
        self
    }

    /// Register a symbol as a member of ambient scope, keyed by its simple name.
    pub(crate) fn with_ambient(mut self, sym: Symbol) -> Self {
        self.ambient
            .entry(sym.name.clone())
            .or_default()
            .push(sym);
        self
    }
}

/// `true` when `kind` names a type-like declaration `types_by_name` should
/// surface.
fn is_type_like(kind: &str) -> bool {
    matches!(
        kind,
        "class" | "struct" | "interface" | "enum" | "trait" | "type" | "object" | "record"
    )
}

impl SymbolLookup for Lookup {
    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(self.by_name.get(name).map(|v| v.as_slice()).unwrap_or(&[]))
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
    fn members_of(&self, parent: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.members
                .get(parent)
                .map(|v| v.as_slice())
                .unwrap_or(&self.empty),
        )
    }
    fn members_of_id(&self, parent_id: i64) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.members_by_id
                .get(&parent_id)
                .map(|v| v.as_slice())
                .unwrap_or(&self.empty),
        )
    }
    fn types_by_name(&self, name: &str) -> SymbolSet<'_> {
        match self.by_name.get(name) {
            Some(v) => SymbolSet::Owned(v.iter().filter(|s| is_type_like(&s.kind)).collect()),
            None => SymbolSet::Borrowed(&self.empty),
        }
    }
    fn in_namespace(&self, namespace: &str) -> Vec<&Symbol> {
        let prefix = format!("{namespace}.");
        self.by_qname
            .values()
            .filter(|s| s.qualified_name.starts_with(&prefix))
            .collect()
    }
    fn has_in_namespace(&self, namespace: &str) -> bool {
        let prefix = format!("{namespace}.");
        self.by_qname
            .keys()
            .any(|q| q.starts_with(&prefix))
    }
    fn in_file(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn field_type_name(&self, qname: &str) -> Option<&str> {
        self.field_types.get(qname).map(|s| s.as_str())
    }
    fn return_type_name(&self, qname: &str) -> Option<&str> {
        self.return_types.get(qname).map(|s| s.as_str())
    }
    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        self.parents.get(class_qname).map(|s| s.as_str())
    }
    fn parent_class_id(&self, child_id: i64) -> Option<i64> {
        self.parents_by_id.get(&child_id).and_then(|v| v.first()).copied()
    }
    fn parent_class_ids(&self, child_id: i64) -> Vec<i64> {
        self.parents_by_id.get(&child_id).cloned().unwrap_or_default()
    }
    fn local_type(&self, name: &str) -> Option<String> {
        self.local_types.get(name).cloned()
    }
    fn enclosing_type_qname(&self, source_qname: &str) -> Option<&str> {
        self.enclosing.get(source_qname).map(|s| s.as_str())
    }
    fn field_type_args(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn generic_params(&self, qname: &str) -> Option<&[String]> {
        self.generics.get(qname).map(|v| v.as_slice())
    }
    fn alias_target(&self, name: &str) -> Option<&AliasTarget> {
        self.aliases.get(name)
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_pairs
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
    fn ambient_symbols(&self, name: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(self.ambient.get(name).map(|v| v.as_slice()).unwrap_or(&self.empty))
    }
    fn type_arena(&self) -> Option<&TypeArena> {
        Some(&self.arena)
    }
    fn symbols_in_package(&self, package_id: i64) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.by_package
                .get(&package_id)
                .map(|v| v.as_slice())
                .unwrap_or(&self.empty),
        )
    }
    fn workspace_package_id(&self, specifier: &str) -> Option<i64> {
        if let Some(&id) = self.workspace_pkgs.get(specifier) {
            return Some(id);
        }
        let mut path = specifier;
        while let Some(slash) = path.rfind('/') {
            path = &path[..slash];
            if let Some(&id) = self.workspace_pkgs.get(path) {
                return Some(id);
            }
        }
        None
    }
    fn is_workspace_declared_name(&self, name: &str) -> bool {
        self.workspace_pkgs.contains_key(name)
    }
}

/// A symbol-index row.
pub(crate) fn sym(id: i64, name: &str, qname: &str, kind: &str, file: &str) -> Symbol {
    Symbol {
        id,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: kind.to_string(),
        visibility: None,
        file_path: Arc::from(file),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

/// An `import name from module` entry.
pub(crate) fn import(name: &str, module: Option<&str>) -> ImportEntry {
    ImportEntry {
        imported_name: name.to_string(),
        module_path: module.map(|s| s.to_string()),
        alias: None,
        is_wildcard: false,
    }
}

/// A file context with the given imports and namespace.
pub(crate) fn file_ctx(imports: Vec<ImportEntry>, ns: Option<&str>) -> FileContext {
    FileContext {
        file_path: "src/main.ts".to_string(),
        language: "typescript".to_string(),
        imports,
        file_namespace: ns.map(|s| s.to_string()),
    }
}

/// A bare `Calls` ref to `target`.
pub(crate) fn call_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

/// A source symbol the ref lives in.
pub(crate) fn source_symbol(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

/// A ref context for `r` in `sym`, with the given scope chain.
pub(crate) fn ref_ctx<'a>(
    r: &'a ExtractedRef,
    sym: &'a ExtractedSymbol,
    scope_chain: Vec<String>,
) -> RefContext<'a> {
    RefContext {
        extracted_ref: r,
        source_symbol: sym,
        scope_chain,
        file_package_id: None,
    }
}

/// Permissive kind predicate — accepts every candidate kind.
pub(crate) fn accept_any(_: EdgeKind, _: &str) -> bool {
    true
}
