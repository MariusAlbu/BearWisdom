//! Configured base recipes and numeric parent edges, never workspace TypeIds.
use super::super::{
    base_receiver::{capture_head, Head},
    head_decl::head_decl_id,
};
use super::*;
use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::indexer::resolve::engine) struct Input {
    pub owner: i64,
    pub head: Head,
    pub args: Vec<Recipe>,
}

pub(super) fn capture(file: &ParsedFile, ids: &SymbolIds, binder: &TypeBinder) -> Vec<Input> {
    binder
        .graph
        .types
        .bases
        .iter()
        .filter_map(|(&slot, base)| {
            let owner = ids.row_id(&file.path, slot)?;
            let (head, args) = base
                .as_ref()
                .map(|(binding, args)| {
                    (
                        capture_head(file, ids, *binding),
                        args.iter().map(|arg| lower(arg, binder)).collect(),
                    )
                })
                .unwrap_or((Head::Unknown, vec![]));
            Some(Input { owner, head, args })
        })
        .collect()
}

impl Input {
    pub(in crate::indexer::resolve::engine) fn materialize(
        &self,
        lookup: &dyn SymbolLookup,
        arena: &TypeArena,
        path: &str,
    ) -> TypeId {
        self.materialize_with_query(lookup, arena, path, &|site| {
            lookup.source_value_type(site).flatten()
        })
    }
    pub(in crate::indexer::resolve::engine) fn materialize_with_query(
        &self,
        lookup: &dyn SymbolLookup,
        arena: &TypeArena,
        path: &str,
        query: &dyn Fn(crate::types::SourceSpan) -> Option<TypeId>,
    ) -> TypeId {
        let unknown = || arena.intern(Type::Unknown);
        if lookup.symbol_by_id(self.owner).is_none() {
            return unknown();
        }
        let target = match self.head {
            Head::Declaration(id) => Some(id),
            Head::Import(binding) => {
                lookup.bound_import(path, crate::indexer::lexical::BindingId(binding), false)
            }
            Head::Unknown => None,
        }
        .map(|id| lookup.canonical_decl_id(id));
        let Some(id) = target
            .filter(|&id| id != lookup.canonical_decl_id(self.owner))
            .filter(|&id| lookup.symbol_by_id(id).is_some_and(|s| s.kind == "class"))
        else {
            return unknown();
        };
        let Some(base) = lookup.declaration_type(arena, id) else {
            return unknown();
        };
        if self.args.is_empty() {
            return base;
        }
        let args = self
            .args
            .iter()
            .map(|r| r.materialize_with_query(lookup, arena, path, query))
            .collect();
        arena.intern(Type::Apply { base, args })
    }
}

#[derive(Default)]
pub(in crate::indexer::resolve::engine) struct Edges {
    parents: FxHashMap<i64, (i64, Vec<TypeId>)>,
    interfaces: FxHashMap<i64, Vec<(i64, Vec<TypeId>)>>,
}

impl Edges {
    pub(in crate::indexer::resolve::engine) fn parent(&self, owner: i64) -> Option<i64> {
        self.parents.get(&owner).map(|p| p.0)
    }
    pub(in crate::indexer::resolve::engine) fn parents(&self, owner: i64) -> Vec<i64> {
        if let Some(parents) = self.interfaces.get(&owner) {
            parents.iter().map(|p| p.0).collect()
        } else {
            self.parent(owner).into_iter().collect()
        }
    }
    pub(in crate::indexer::resolve::engine) fn args(&self, owner: i64, parent: i64) -> &[TypeId] {
        if let Some(parents) = self.interfaces.get(&owner) {
            return parents
                .iter()
                .find(|p| p.0 == parent)
                .map(|p| p.1.as_slice())
                .unwrap_or(&[]);
        }
        self.parents
            .get(&owner)
            .filter(|p| p.0 == parent)
            .map(|p| p.1.as_slice())
            .unwrap_or(&[])
    }
    pub(in crate::indexer::resolve::engine) fn install_interfaces(
        &mut self,
        pending: FxHashMap<i64, Vec<TypeId>>,
        info: &mut FxHashMap<i64, TypeInfo>,
        arena: &TypeArena,
    ) {
        for (owner, types) in pending {
            let parents = types
                .iter()
                .filter_map(|&ty| {
                    Some((
                        head_decl_id(arena, ty)?,
                        match arena.get(ty) {
                            Type::Apply { args, .. } => args,
                            _ => vec![],
                        },
                    ))
                })
                .collect();
            let base = match types.as_slice() {
                [] => None,
                [ty] => Some(*ty),
                _ => Some(arena.intern(Type::Intersection(types))),
            };
            info.entry(owner).or_default().base_type_id = base;
            self.interfaces.insert(owner, parents);
        }
    }
    pub(in crate::indexer::resolve::engine) fn install(
        pending: Vec<(i64, TypeId)>,
        info: &mut FxHashMap<i64, TypeInfo>,
        arena: &TypeArena,
    ) -> Self {
        let unknown = arena.intern(Type::Unknown);
        let mut types = FxHashMap::default();
        for (owner, ty) in pending {
            types
                .entry(owner)
                .and_modify(|prior| {
                    if *prior != ty {
                        *prior = unknown;
                    }
                })
                .or_insert(ty);
        }
        let mut edges = Self::default();
        for (&owner, &ty) in &types {
            let ty = if acyclic(owner, &types, arena) {
                ty
            } else {
                unknown
            };
            if let Some(target) = head_decl_id(arena, ty) {
                let args = match arena.get(ty) {
                    Type::Apply { args, .. } => args,
                    _ => vec![],
                };
                edges.parents.insert(owner, (target, args));
            }
            info.entry(owner).or_default().base_type_id = Some(ty);
        }
        edges
    }
}

fn acyclic(mut owner: i64, types: &FxHashMap<i64, TypeId>, arena: &TypeArena) -> bool {
    let mut seen = FxHashSet::default();
    for _ in 0..64 {
        if !seen.insert(owner) {
            return false;
        }
        let Some(parent) = types.get(&owner).and_then(|&ty| head_decl_id(arena, ty)) else {
            return true;
        };
        owner = parent;
    }
    false
}

#[cfg(test)]
#[path = "program_bases_tests.rs"]
mod tests;
