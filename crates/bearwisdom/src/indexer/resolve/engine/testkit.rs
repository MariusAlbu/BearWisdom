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

use crate::indexer::resolve::engine::contract::{Symbol, SymbolLookup, SymbolSet};

pub(crate) use super::testkit_fixtures::*;
use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::types::{
    intern_alias_target, AliasTarget, AliasTargetIds, EdgeKind, ExtractedRef, ExtractedSymbol,
    SymbolKind, Visibility,
};

/// Synthetic symbol index. Register symbols with `with`, members with
/// `with_member`; everything else is empty.
pub(crate) struct Lookup {
    generics_of: FxHashMap<i64, Vec<String>>,
    parent_arg_ids_of: FxHashMap<(i64, i64), Vec<TypeId>>,
    empty: Vec<Symbol>,
    empty_pairs: Vec<(String, String)>,
    by_name: FxHashMap<String, Vec<Symbol>>,
    by_qname: FxHashMap<String, Symbol>,
    by_qname_all: FxHashMap<String, Vec<Symbol>>,
    /// Id → symbol record. Backs `symbol_by_id` for the id-keyed parent climb.
    by_id: FxHashMap<i64, Symbol>,
    members: FxHashMap<String, Vec<Symbol>>,
    /// Id-keyed members: parent symbol id → member symbols. The id-keyed
    /// counterpart of `members`, exercising the chain walker's identity path.
    members_by_id: FxHashMap<i64, Vec<Symbol>>,
    member_index: super::member_index::MemberIndex,
    generics: FxHashMap<String, Vec<String>>,
    field_types: FxHashMap<String, String>,
    field_type_ids: FxHashMap<String, TypeId>,
    return_types: FxHashMap<String, String>,
    /// Id-keyed return types — the collision-free counterpart of
    /// `return_types`, for same-qname overload rows with distinct yields.
    return_types_by_id: FxHashMap<i64, String>,
    parents: FxHashMap<String, Vec<String>>,
    /// Id-keyed inherits: child symbol id → ALL parent symbol ids. The id-keyed
    /// counterpart of `parents`; a child may have several supertypes.
    parents_by_id: FxHashMap<i64, Vec<i64>>,
    /// Generic args on an `extends`/`implements` edge: `(child_head, parent_head)`
    /// → args. Backs `parent_class_args` for the supertype-arg binding tests.
    inherits_args: FxHashMap<(String, String), Vec<String>>,
    pub(crate) local_types: FxHashMap<String, String>,
    pub(crate) local_callable_heads: FxHashMap<String, String>,
    enclosing: FxHashMap<String, String>,
    aliases: FxHashMap<String, AliasTargetIds>,
    /// Id-keyed alias targets — the collision-free counterpart of `aliases`.
    aliases_by_id: FxHashMap<i64, AliasTargetIds>,
    /// File path → its re-export entries `(original_name, source_module)`, with
    /// `"*"` for an `export * from 'm'` wildcard. Backs `reexports_from`.
    reexports: FxHashMap<String, Vec<(String, String)>>,
    ambient: FxHashMap<String, Vec<Symbol>>,
    /// Package id → symbols, the id-keyed counterpart of the real `by_package`
    /// map. Backs `symbols_in_package` for workspace-scoped rule tests.
    by_package: FxHashMap<i64, Vec<Symbol>>,
    /// Declared workspace-package specifier → package id, backing
    /// `workspace_package_id` / `is_workspace_declared_name`.
    workspace_pkgs: FxHashMap<String, i64>,
    /// Manifest-declared implicit namespace imports, backing
    /// `implicit_wildcard_namespaces` (workspace-wide only in tests).
    implicit_namespaces: Vec<String>,
    /// Workspace arena the chain walker interns roots / yields into. Mirrors the
    /// real `Compilation`, which owns one; the chain walk declines without it.
    arena: TypeArena,
}

mod builders;

/// `true` when `kind` names a type-like declaration `types_by_name` should
/// surface.
// Mirrors the production predicate (`contract::is_type_like_kind`), extended
// with the kinds only synthetic fixtures use — the fixture must classify a
// `type_alias` the way `SymbolIndex` does, or alias-aware walks diverge
// between test and prod.
fn is_type_like(kind: &str) -> bool {
    super::contract::is_type_like_kind(kind)
        || matches!(kind, "trait" | "type" | "object" | "record")
}

impl SymbolLookup for Lookup {
    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(self.by_name.get(name).map(|v| v.as_slice()).unwrap_or(&[]))
    }
    fn by_qualified_name(&self, qname: &str) -> Option<&Symbol> {
        self.by_qname.get(qname)
    }
    fn symbol_by_id(&self, id: i64) -> Option<&Symbol> {
        self.by_id.get(&id)
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
    fn member_index(&self) -> Option<&super::member_index::MemberIndex> {
        Some(&self.member_index)
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
        self.by_qname.keys().any(|q| q.starts_with(&prefix))
    }
    fn in_file(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn field_type_name(&self, qname: &str) -> Option<&str> {
        self.field_types.get(qname).map(|s| s.as_str())
    }
    fn field_type_id(&self, qname: &str) -> Option<TypeId> {
        self.field_type_ids.get(qname).copied()
    }
    fn return_type_name(&self, qname: &str) -> Option<&str> {
        self.return_types.get(qname).map(|s| s.as_str())
    }
    fn return_type_id_of(&self, symbol_id: i64) -> Option<TypeId> {
        self.return_types_by_id
            .get(&symbol_id)
            .map(|s| self.arena.intern_type_str(s))
    }
    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        self.parents
            .get(class_qname)
            .and_then(|v| v.first())
            .map(|s| s.as_str())
    }
    fn parent_class_qnames(&self, class_qname: &str) -> &[String] {
        self.parents
            .get(class_qname)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
    fn generic_params_of(&self, symbol_id: i64) -> Option<Vec<String>> {
        self.generics_of.get(&symbol_id).cloned()
    }

    fn parent_class_arg_ids_of(&self, child_id: i64, parent_id: i64) -> &[TypeId] {
        self.parent_arg_ids_of
            .get(&(child_id, parent_id))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn parent_class_id(&self, child_id: i64) -> Option<i64> {
        self.parents_by_id
            .get(&child_id)
            .and_then(|v| v.first())
            .copied()
    }
    fn parent_class_ids(&self, child_id: i64) -> Vec<i64> {
        self.parents_by_id
            .get(&child_id)
            .cloned()
            .unwrap_or_default()
    }
    fn parent_class_args(&self, child_head: &str, parent_head: &str) -> &[String] {
        self.inherits_args
            .get(&(child_head.to_string(), parent_head.to_string()))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
    fn enclosing_type_qname(&self, source_qname: &str) -> Option<&str> {
        self.enclosing.get(source_qname).map(|s| s.as_str())
    }
    fn generic_params(&self, qname: &str) -> Option<Vec<String>> {
        self.generics.get(qname).cloned()
    }
    fn alias_target(&self, name: &str) -> Option<&AliasTargetIds> {
        self.aliases.get(name)
    }
    fn alias_target_by_id(&self, id: i64) -> Option<&AliasTargetIds> {
        self.aliases_by_id.get(&id)
    }
    fn reexports_from(&self, file_path: &str) -> &[(String, String)] {
        self.reexports
            .get(file_path)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty_pairs)
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
    fn ambient_symbols(&self, name: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.ambient
                .get(name)
                .map(|v| v.as_slice())
                .unwrap_or(&self.empty),
        )
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
    fn implicit_wildcard_namespaces(&self, _package_id: Option<i64>) -> &[String] {
        &self.implicit_namespaces
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
