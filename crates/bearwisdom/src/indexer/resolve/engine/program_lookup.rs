//! Source selection over a configured view. String-query methods deliberately
//! have no workspace fallback; runtime consumers must carry source-bound IDs.
use super::*;
use crate::indexer::resolve::engine::contract::{
    flow_cache::LocalReference, FlowCacheLookup, Symbol, SymbolSet,
};
use crate::type_checker::core::types::TypeId;

pub(in crate::indexer::resolve::engine) fn select_method(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    receiver: TypeId,
    name: crate::indexer::resolve::engine::member_index::MemberNameId,
) -> Result<crate::indexer::resolve::engine::contract::flow_cache::BoundMethod, ()> {
    use crate::indexer::resolve::engine::{
        chain::Receiver,
        contract::member_applicability,
        head_decl,
        member_selection::{select_typed, Selection},
    };
    let receiver = member_applicability::expand(lookup, arena, receiver).ok_or(())?;
    let receiver = super::project_intrinsic(lookup, arena, receiver).ok_or(())?;
    let owner = head_decl::head_decl_id(arena, receiver).ok_or(())?;
    let Selection::Unique(declaration) =
        select_typed(lookup, arena, Receiver::new(receiver, owner), name, &|_| {
            true
        })
    else {
        return Err(());
    };
    Ok(
        crate::indexer::resolve::engine::contract::flow_cache::BoundMethod {
            declaration,
            receiver,
            adjusted: receiver,
            bindings: FxHashMap::default(),
        },
    )
}

/// The source selected a lexical private declaration. Check its receiver brand
/// by containment/parent IDs, without selecting another same-spelled member.
pub(in crate::indexer::resolve::engine) fn select_private(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    receiver: TypeId,
    declaration: i64,
) -> Result<crate::indexer::resolve::engine::contract::flow_cache::BoundMethod, ()> {
    use crate::indexer::resolve::engine::{contract::member_applicability, head_decl};
    let receiver = member_applicability::expand(lookup, arena, receiver).ok_or(())?;
    lookup.symbol_by_id(declaration).ok_or(())?;
    let mut pending = vec![head_decl::head_decl_id(arena, receiver).ok_or(())?];
    let mut seen = rustc_hash::FxHashSet::default();
    while let Some(owner) = pending.pop() {
        if seen.len() >= 64 {
            return Err(());
        }
        if !seen.insert(owner) {
            continue;
        }
        if lookup
            .members_of_id(owner)
            .iter()
            .any(|member| member.id == declaration)
        {
            return Ok(
                crate::indexer::resolve::engine::contract::flow_cache::BoundMethod {
                    declaration,
                    receiver,
                    adjusted: receiver,
                    bindings: FxHashMap::default(),
                },
            );
        }
        pending.extend(lookup.parent_class_ids(owner));
    }
    Err(())
}

pub(in crate::indexer::resolve::engine) struct Lookup<'a> {
    pub(super) tree: &'a Compilation,
    pub(super) view: &'a View,
    pub(super) source: Option<&'a Source>,
}
impl Lookup<'_> {
    pub(in crate::indexer::resolve::engine) fn object_member(
        &self,
        receiver: TypeId,
        name: super::super::member_index::MemberNameId,
    ) -> Option<Result<super::super::contract::flow_cache::ObjectMember, ()>> {
        super::object_members::read(self, receiver, name)
    }
    pub(in crate::indexer::resolve::engine) fn source_member_id(
        &self,
        name: NameId,
    ) -> Option<super::super::member_index::MemberNameId> {
        self.source?.member_names.get(&name).copied()
    }
    pub(in crate::indexer::resolve::engine) fn effective_member(
        &self,
        owner: i64,
        member: i64,
    ) -> Option<&super::merge_proof::heritage::EffectiveMember> {
        self.symbol_by_id(owner)?;
        self.symbol_by_id(member)?;
        let owner = self.canonical_decl_id(owner);
        let index = self.view.effective_members.get(&(owner, member))?;
        self.view.effective_surfaces.get(&owner)?.get(*index)
    }
    pub(in crate::indexer::resolve::engine) fn nominal_surface(
        &self,
        owner: i64,
    ) -> Option<&super::nominal::Surface> {
        self.symbol_by_id(owner)?;
        self.view
            .nominal_surfaces
            .get(&self.canonical_decl_id(owner))
    }
    pub(in crate::indexer::resolve::engine) fn generic_constraint(
        &self,
        parameter: crate::type_checker::core::types::GenericParamId,
    ) -> Option<Option<TypeId>> {
        self.view.generic_constraints.get(&parameter).copied()
    }
    pub(in crate::indexer::resolve::engine) fn computed_key(
        &self,
        site: crate::types::SourceSpan,
    ) -> Option<Option<TypeId>> {
        Some(
            self.source
                .and_then(|source| source.computed_keys.get(&site).copied().flatten()),
        )
    }
    pub(in crate::indexer::resolve::engine) fn signature(
        &self,
        id: source_signatures::SignatureId,
    ) -> Option<&source_signatures::Bound> {
        self.source?.signatures.get(&id)
    }
    pub(in crate::indexer::resolve::engine) fn global_value(&self, name: NameId) -> LocalReference {
        let declaration = self
            .source
            .and_then(|s| s.globals.get(&(name, false)))
            .copied()
            .flatten();
        let symbol = declaration.and_then(|id| self.symbol_by_id(id));
        LocalReference {
            declaration: symbol.map(|s| s.id),
            kind: symbol
                .and_then(|s| s.kind.parse().ok())
                .unwrap_or(crate::types::SymbolKind::Variable),
            value_type: declaration.and_then(|id| self.field_type_id_of(id)),
            type_args: vec![],
            callable: None,
        }
    }
}
impl FlowCacheLookup for Lookup<'_> {
    fn evaluated_receiver(&self, receiver: TypeId) -> Option<Option<TypeId>> {
        Some(self.type_arena().and_then(|arena| {
            super::merge_proof::types::Relation {
                lookup: self,
                arena,
            }
            .canonical(receiver, 0)
        }))
    }
    fn object_member_type(&self, receiver: TypeId, member: i64) -> Option<Option<TypeId>> {
        super::object_members::value(self, receiver, member)
    }
    fn source_initializer_type(
        &self,
        owner: crate::types::SourceSpan,
        target: crate::types::SourceSpan,
    ) -> Option<Option<TypeId>> {
        Some((|| {
            let source = self.source?;
            let signature = source_signatures::SignatureId(owner);
            if source
                .initializer_targets
                .get(&signature)
                .copied()
                .flatten()
                != Some(target)
            {
                return None;
            }
            source.initializers.get(&signature).copied().flatten()
        })())
    }
    fn source_callable_origin(
        &self,
        owner: crate::types::SourceSpan,
    ) -> Option<crate::type_checker::core::types::CallableOrigin> {
        self.signature(source_signatures::SignatureId(owner))?;
        Some(crate::type_checker::core::types::CallableOrigin::new(
            self.view.context,
            self.source?.identity?.ordinal(),
            owner,
        ))
    }
    fn receiver_member_info(&self, owner: i64, member: i64) -> Option<&TypeInfo> {
        self.effective_member(owner, member).map(|m| &m.info)
    }
    fn source_value_type(&self, site: crate::types::SourceSpan) -> Option<Option<TypeId>> {
        Some(
            self.source
                .and_then(|source| source.value_queries.get(&site).copied().flatten()),
        )
    }
    fn source_unique_symbol(&self, declaration: crate::types::SourceSpan) -> Option<TypeId> {
        self.source?.unique_symbols.get(&declaration).copied()
    }
    fn intrinsic_member_type(
        &self,
        kind: crate::type_checker::core::types::Intrinsic,
    ) -> Option<TypeId> {
        (self as &dyn SymbolLookup).declaration_type(
            self.tree.type_arena()?,
            *self.source?.intrinsic_members.get(&kind)?,
        )
    }
    fn nominal_context(&self) -> Option<NominalContextId> {
        Some(self.view.context)
    }
    fn source_global_type(&self, name: NameId) -> Option<Option<i64>> {
        Some(
            self.source
                .and_then(|s| s.globals.get(&(name, true)))
                .copied()
                .flatten(),
        )
    }
    fn source_signature_parameter(
        &self,
        owner: crate::types::SourceSpan,
        index: usize,
    ) -> Option<crate::type_checker::core::types::GenericParamId> {
        self.signature(source_signatures::SignatureId(owner))?
            .generic_parameters
            .get(index)
            .copied()
    }
}
impl SymbolLookup for Lookup<'_> {
    fn by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
    }
    fn by_qualified_name(&self, _: &str) -> Option<&Symbol> {
        None
    }
    fn members_of(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
    }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
    }
    fn in_namespace(&self, _: &str) -> Vec<&Symbol> {
        vec![]
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
    }
    fn field_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn generic_params(&self, _: &str) -> Option<Vec<String>> {
        None
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &[]
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
    fn symbol_by_id(&self, id: i64) -> Option<&Symbol> {
        self.view
            .canonical
            .contains_key(&id)
            .then(|| self.tree.symbol_by_id(id))
            .flatten()
    }
    fn canonical_decl_id(&self, id: i64) -> i64 {
        self.view.canonical.get(&id).copied().unwrap_or(id)
    }
    fn canonical_type_info(&self, id: i64) -> Option<&TypeInfo> {
        self.symbol_by_id(id)?;
        self.view.info.get(&self.canonical_decl_id(id))
    }
    fn member_index(&self) -> Option<&crate::indexer::resolve::engine::member_index::MemberIndex> {
        Some(&self.view.members)
    }
    fn parent_class_id(&self, child: i64) -> Option<i64> {
        self.symbol_by_id(child)?;
        self.view
            .bases
            .parent(self.canonical_decl_id(child))
            .filter(|&id| self.symbol_by_id(id).is_some())
    }
    fn parent_class_ids(&self, child: i64) -> Vec<i64> {
        if self.symbol_by_id(child).is_none() {
            return vec![];
        }
        self.view
            .bases
            .parents(self.canonical_decl_id(child))
            .into_iter()
            .filter(|&id| self.symbol_by_id(id).is_some())
            .collect()
    }
    fn parent_class_arg_ids_of(&self, child: i64, parent: i64) -> &[TypeId] {
        if self.symbol_by_id(child).is_none() || self.symbol_by_id(parent).is_none() {
            return &[];
        }
        self.view.bases.args(
            self.canonical_decl_id(child),
            self.canonical_decl_id(parent),
        )
    }
    fn members_of_id(&self, id: i64) -> SymbolSet<'_> {
        SymbolSet::Owned(
            self.view
                .children
                .get(&self.canonical_decl_id(id))
                .into_iter()
                .flatten()
                .filter_map(|&id| self.symbol_by_id(id))
                .collect(),
        )
    }
    fn type_arena(&self) -> Option<&TypeArena> {
        self.tree.type_arena()
    }
    fn return_type_id_of(&self, id: i64) -> Option<TypeId> {
        self.canonical_type_info(id)?.return_type_id
    }
    fn field_type_id_of(&self, id: i64) -> Option<TypeId> {
        self.canonical_type_info(id)?.field_type_id
    }
    fn generic_return_of(
        &self,
        id: i64,
    ) -> Option<&crate::indexer::resolve::engine::contract::generic_return::GenericReturn> {
        self.canonical_type_info(id)?.generic_return.as_ref()
    }
    fn bound_import(
        &self,
        file: &str,
        binding: crate::indexer::lexical::BindingId,
        domain: bool,
    ) -> Option<i64> {
        self.view
            .modules
            .binding(file, binding, domain)
            .declaration()
            .filter(|&id| self.symbol_by_id(id).is_some())
    }
    fn bound_import_namespace(
        &self,
        file: &str,
        binding: crate::indexer::lexical::BindingId,
    ) -> bool {
        self.view
            .modules
            .binding(file, binding, false)
            .namespace()
            .is_some()
    }
    fn bound_import_overloads(
        &self,
        file: &str,
        binding: crate::indexer::lexical::BindingId,
    ) -> &[i64] {
        self.view.modules.overloads(file, binding)
    }
    fn resolve_module_from(&self, file: &str, spec: &str) -> Option<&str> {
        self.tree.resolve_module_from(file, spec)
    }
    fn package_id_for_file(&self, file: &str) -> Option<i64> {
        self.tree.package_id_for_file(file)
    }
    fn resolve_path_alias(&self, package: Option<i64>, spec: &str) -> Option<String> {
        self.tree.resolve_path_alias(package, spec)
    }
    fn enclosing_type_id_of(&self, id: i64) -> Option<i64> {
        self.tree
            .enclosing_type_id_of(id)
            .filter(|&id| self.symbol_by_id(id).is_some())
            .map(|id| self.canonical_decl_id(id))
    }
}

#[cfg(test)]
#[path = "program_lookup_tests.rs"]
mod tests;
