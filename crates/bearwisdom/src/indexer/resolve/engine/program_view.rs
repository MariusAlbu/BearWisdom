//! Immutable configured signature/member environments. Physical rows are kept
//! for navigation; canonical owners and all type parameters belong to a view.
use super::program_types::source_signatures;
use super::{
    compilation::Compilation,
    contract::{SymbolLookup, TypeInfo},
    module_graph::ModuleGraph,
    program_graph::{ProgramId, Result as BindingResult, SourceInstanceId},
};
use crate::indexer::lexical::NameId;
use crate::type_checker::core::types::{GenericParamData, NominalContextId, TypeArena};
use rustc_hash::FxHashMap;

#[path = "program_lookup.rs"]
mod lookup;
pub(super) use lookup::Lookup;
pub(super) use lookup::{select_method, select_private};

#[path = "program_overload_calls.rs"]
pub(super) mod overload_calls;

#[path = "program_callback_bodies.rs"]
pub(super) mod callback_bodies;

#[path = "program_constructor_arguments.rs"]
mod call_arguments;

#[path = "program_call_selection.rs"]
mod call_selection;
#[path = "program_constructor_selection.rs"]
mod constructors;

#[path = "intrinsic_members.rs"]
mod intrinsic_members;
pub(super) use intrinsic_members::project as project_intrinsic;

#[path = "program_array_types.rs"]
pub(super) mod arrays;
#[path = "program_compiler_intrinsics.rs"]
mod compiler_intrinsics;
#[path = "program_computed_keys.rs"]
pub(super) mod computed_keys;
#[path = "program_generic_groups.rs"]
mod generic_groups;
#[path = "program_merge_proof.rs"]
mod merge_proof;
#[path = "program_nominal_surfaces.rs"]
pub(super) mod nominal;
#[path = "program_object_members.rs"]
mod object_members;
#[path = "program_source_members.rs"]
mod source_members;
#[path = "program_value_queries.rs"]
mod value_queries;

#[derive(Default)]
pub(super) struct Source {
    member_names: FxHashMap<NameId, super::member_index::MemberNameId>,
    identity: Option<SourceInstanceId>,
    initializers:
        FxHashMap<source_signatures::SignatureId, Option<crate::type_checker::core::types::TypeId>>,
    initializer_targets:
        FxHashMap<source_signatures::SignatureId, Option<crate::types::SourceSpan>>,
    constructor_calls: FxHashMap<source_signatures::SignatureId, Option<constructors::Call>>,
    value_queries:
        FxHashMap<crate::types::SourceSpan, Option<crate::type_checker::core::types::TypeId>>,
    computed_keys:
        FxHashMap<crate::types::SourceSpan, Option<crate::type_checker::core::types::TypeId>>,
    unique_symbols: FxHashMap<crate::types::SourceSpan, crate::type_checker::core::types::TypeId>,
    intrinsic_members: FxHashMap<crate::type_checker::core::types::Intrinsic, i64>,
    content_hash: String,
    globals: FxHashMap<(NameId, bool), Option<i64>>,
    signatures: FxHashMap<source_signatures::SignatureId, source_signatures::Bound>,
}
pub(super) struct View {
    generic_inputs: generic_groups::Index,
    generic_declarations: FxHashMap<i64, generic_groups::Group>,
    arrays: arrays::Roles,
    source_members: source_members::Index,
    objects: object_members::Index,
    source_binding_order: Option<FxHashMap<SourceInstanceId, usize>>,
    callable_policy: Option<crate::indexer::programs::CallablePolicy>,
    compiler_intrinsics: Option<crate::indexer::programs::CompilerIntrinsicPolicy>,
    query_members: FxHashMap<
        (i64, super::member_index::MemberNameId),
        Option<crate::type_checker::core::types::TypeId>,
    >,
    pub key_names:
        FxHashMap<crate::type_checker::core::types::TypeId, super::member_index::MemberNameId>,
    pub name_keys:
        FxHashMap<super::member_index::MemberNameId, crate::type_checker::core::types::TypeId>,
    pub effective_members: FxHashMap<(i64, i64), usize>,
    pub effective_surfaces: FxHashMap<i64, Vec<merge_proof::heritage::EffectiveMember>>,
    pub nominal_surfaces: FxHashMap<i64, nominal::Surface>,
    callable_members: FxHashMap<i64, Option<(SourceInstanceId, source_signatures::SignatureId)>>,
    pub generic_constraints: FxHashMap<
        crate::type_checker::core::types::GenericParamId,
        Option<crate::type_checker::core::types::TypeId>,
    >,
    pub bases: super::program_types::bases::Edges,
    pub modules: ModuleGraph,
    pub context: NominalContextId,
    pub canonical: FxHashMap<i64, i64>,
    pub info: FxHashMap<i64, TypeInfo>,
    pub members: super::member_index::MemberIndex,
    pub children: FxHashMap<i64, Vec<i64>>,
    sources: FxHashMap<SourceInstanceId, Source>,
}
pub(super) struct Store {
    legacy_intrinsics: FxHashMap<crate::type_checker::core::types::Intrinsic, Option<i64>>,
    views: FxHashMap<ProgramId, View>,
    paths: FxHashMap<String, Vec<(ProgramId, SourceInstanceId)>>,
    unavailable: View,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            legacy_intrinsics: Default::default(),
            views: FxHashMap::default(),
            paths: FxHashMap::default(),
            unavailable: View::empty(),
        }
    }
}
impl Store {
    pub(super) fn legacy_intrinsic(
        &self,
        kind: crate::type_checker::core::types::Intrinsic,
    ) -> Option<i64> {
        self.legacy_intrinsics.get(&kind).copied().flatten()
    }
    pub(super) fn build(modules: &ModuleGraph, tree: &Compilation, arena: &TypeArena) -> Self {
        let mut store = Self::default();
        store.legacy_intrinsics = intrinsic_members::bind_legacy(modules, tree);
        for program in modules.programs.programs() {
            let sources = modules.programs.sources(program);
            for (path, source) in &sources {
                store
                    .paths
                    .entry(path.clone())
                    .or_default()
                    .push((program, *source));
            }
            store.views.insert(
                program,
                merge_proof::build(program, &sources, modules, tree, arena),
            );
        }
        store
    }
    pub(super) fn for_file<'a>(&'a self, tree: &'a Compilation, path: &str) -> Option<Lookup<'a>> {
        let candidates = self.paths.get(&super::module_paths::normalize(path))?;
        Some(match candidates.as_slice() {
            [(program, source)] => Lookup {
                tree,
                view: &self.views[program],
                source: self.views[program].sources.get(source),
            },
            _ => Lookup {
                tree,
                view: &self.unavailable,
                source: None,
            },
        })
    }
    pub(super) fn for_source<'a>(
        &'a self,
        tree: &'a Compilation,
        file: &crate::types::ParsedFile,
    ) -> Option<Lookup<'a>> {
        let selected = self.for_file(tree, &file.path)?;
        Some(
            if selected
                .source
                .is_some_and(|source| source.content_hash == file.content_hash)
            {
                selected
            } else {
                Lookup {
                    tree,
                    view: &self.unavailable,
                    source: None,
                }
            },
        )
    }
}
impl View {
    fn empty() -> Self {
        let source_binding_order = None;
        Self {
            generic_inputs: Default::default(),
            generic_declarations: Default::default(),
            compiler_intrinsics: None,
            objects: Default::default(),
            arrays: Default::default(),
            source_members: Default::default(),
            callable_members: FxHashMap::default(),
            callable_policy: None,
            query_members: FxHashMap::default(),
            key_names: FxHashMap::default(),
            name_keys: FxHashMap::default(),
            effective_members: FxHashMap::default(),
            effective_surfaces: FxHashMap::default(),
            nominal_surfaces: FxHashMap::default(),
            generic_constraints: FxHashMap::default(),
            modules: ModuleGraph::default(),
            context: NominalContextId::fresh(),
            canonical: FxHashMap::default(),
            info: FxHashMap::default(),
            source_binding_order,
            bases: Default::default(),
            members: Default::default(),
            children: FxHashMap::default(),
            sources: FxHashMap::default(),
        }
    }
    fn build(
        program: ProgramId,
        sources: &[(String, SourceInstanceId)],
        modules: &ModuleGraph,
        tree: &Compilation,
        arena: &TypeArena,
        programs: &super::program_graph::Graph,
        excluded: &rustc_hash::FxHashSet<i64>,
    ) -> Self {
        let mut view = Self::empty();
        let Some(context) = programs.nominal_context(program) else {
            return view;
        };
        view.context = context;
        view.callable_policy = programs.callable_policy(program);
        view.compiler_intrinsics = programs.compiler_intrinsics(program);
        view.source_binding_order = programs.source_binding_order(program);
        // Every provider must retain source recipes. Missing caches are not
        // permission to borrow old workspace signatures or generic parameters.
        let inputs = sources
            .iter()
            .map(|(path, source)| {
                Some((
                    path,
                    source,
                    modules.inputs.get(path)?.globals.as_ref()?.types.as_ref()?,
                ))
            })
            .collect::<Option<Vec<_>>>();
        let Some(inputs) = inputs else {
            return view;
        };
        for (_, _, input) in &inputs {
            for &row in &input.declarations {
                if tree.symbol_by_id(row).is_none() {
                    return Self::empty();
                }
                view.canonical.insert(row, tree.canonical_decl_id(row));
                view.info.insert(row, TypeInfo::default());
            }
        }
        // A lexical BindingId supplies the local candidate identity. This is
        // still a private view: generic and member proofs may revoke it below.
        for (_, _, input) in &inputs {
            for group in &input.interface_groups {
                let Some(&owner) = group.first() else {
                    continue;
                };
                if !group.iter().all(|row| view.canonical.contains_key(row)) {
                    return Self::empty();
                }
                for &row in group {
                    view.canonical.insert(row, owner);
                }
            }
        }
        for group in programs.groups(program) {
            let Some(first) = group.parts.first() else {
                continue;
            };
            for part in &group.parts {
                view.canonical.insert(part.declaration, first.declaration);
            }
        }
        let rejected: rustc_hash::FxHashSet<_> =
            programs.rejected(program).iter().copied().collect();
        view.canonical.retain(|row, owner| {
            !rejected.contains(row)
                && !rejected.contains(owner)
                && !excluded.contains(row)
                && !excluded.contains(owner)
        });
        for (_, _, input) in &inputs {
            for (row, parameters) in &input.parameters {
                let Some(&owner) = view.canonical.get(row) else {
                    continue;
                };
                let info = view.info.entry(owner).or_default();
                let prior = info.generic_param_ids.len();
                if prior >= parameters.len() {
                    continue;
                }
                info.generic_param_ids
                    .extend(parameters.iter().skip(prior).map(|(name, kind)| {
                        arena.intern_generic(GenericParamData {
                            name: name.clone(),
                            kind: *kind,
                            owner_symbol_index: 0,
                            bound: None,
                        })
                    }));
                info.generic_param_default_ids = vec![None; parameters.len()];
            }
        }
        let selected = Lookup {
            tree,
            view: &view,
            source: None,
        };
        let Some(bound_modules) = modules.for_program(program, &selected) else {
            return Self::empty();
        };
        view.modules = bound_modules;
        for (path, &source, input) in &inputs {
            let mut bound = Source {
                identity: Some(source),
                content_hash: modules.inputs[*path].content_hash.clone(),
                ..Default::default()
            };
            bound.unique_symbols = input
                .unique_symbols
                .iter()
                .map(|&site| {
                    (
                        site,
                        arena.intern(crate::type_checker::core::types::Type::UniqueSymbol(
                            crate::type_checker::core::types::UniqueSymbol::new(
                                view.context,
                                source.ordinal(),
                                site,
                            ),
                        )),
                    )
                })
                .collect();
            bound.signatures =
                source_signatures::allocate(input, &view.canonical, &view.info, arena);
            for initializer in &input.initializers {
                bound
                    .initializer_targets
                    .entry(initializer.signature)
                    .and_modify(|old| *old = None)
                    .or_insert(initializer.target);
            }
            callback_bodies::inventory(&mut view, source, input);
            for (name, spelling) in &input.names {
                for domain in [false, true] {
                    let target = programs.name(program, spelling).and_then(|name| {
                        match programs.global(program, name, domain) {
                            BindingResult::Bound(group) => {
                                programs.group(group)?.parts.first().map(|p| p.declaration)
                            }
                            _ => None,
                        }
                    });
                    bound.globals.insert((*name, domain), target);
                }
            }
            bound.intrinsic_members = input
                .intrinsic_members
                .iter()
                .filter_map(|(kind, name)| {
                    bound
                        .globals
                        .get(&(*name, true))
                        .copied()
                        .flatten()
                        .map(|row| (*kind, row))
                })
                .collect();
            view.sources.insert(source, bound);
        }
        view.generic_inputs = generic_groups::Index::capture(&view, &inputs);
        view.arrays = arrays::Roles::bind(&view, &inputs);
        view.members = tree
            .member_index()
            .map(|index| index.project(&view.canonical))
            .unwrap_or_default();
        view.source_members = source_members::Index::capture(&mut view, &inputs);
        view.objects = object_members::Index::capture(&view, &inputs, arena);
        for (&row, &owner) in &view.canonical {
            let children = view.children.entry(owner).or_default();
            children.extend(
                tree.members_of_id(row)
                    .into_iter()
                    .filter(|s| view.canonical.contains_key(&s.id))
                    .map(|s| s.id),
            );
            children.sort_unstable();
            children.dedup();
        }
        value_queries::bind(&mut view, &inputs, tree, arena);
        view.materialize(&inputs, tree, arena);
        nominal::bind(&mut view, sources, modules, tree, arena);
        view
    }

    fn materialize(
        &mut self,
        inputs: &computed_keys::Sources,
        tree: &Compilation,
        arena: &TypeArena,
    ) {
        let view = self;
        // Each round replaces derived facts, retaining only allocated source IDs.
        view.generic_constraints.clear();
        view.effective_members.clear();
        view.effective_surfaces.clear();
        if let Some(index) = tree.member_index() {
            view.members.restore_projection(index, &view.canonical);
        }
        view.source_members.project(&mut view.members);
        for info in view.info.values_mut() {
            info.base_type_id = None;
        }
        let mut pending = Vec::new();
        let mut bases = Vec::new();
        let mut signatures = Vec::new();
        for (path, source, input) in inputs {
            let lookup = Lookup {
                tree,
                view: &view,
                source: view.sources.get(source),
            };
            for signature in &input.source_signatures {
                let mut bound = view.sources[source].signatures[&signature.id].clone();
                source_signatures::materialize(signature, &mut bound, &lookup, arena, path);
                if signature.result.is_none() {
                    bound.result = view.sources[source]
                        .initializers
                        .get(&signature.id)
                        .copied()
                        .flatten();
                }
                signatures.push((**source, signature.id, signature.declaration, bound));
            }
            bases.extend(
                input
                    .bases
                    .iter()
                    .filter(|base| view.canonical.contains_key(&base.owner))
                    .map(|base| {
                        (
                            lookup.canonical_decl_id(base.owner),
                            base.materialize(&lookup, arena, path),
                        )
                    }),
            );
            for signature in &input.signatures {
                pending.push((
                    signature.declaration,
                    signature.slot,
                    compiler_intrinsics::materialize(signature, input, &lookup, arena)
                        .unwrap_or_else(|| {
                            signature
                                .recipes
                                .iter()
                                .map(|r| r.materialize(&lookup, arena, path))
                                .collect()
                        }),
                ));
            }
            for initializer in input.initializers.iter().filter(|i| !i.annotated) {
                if let Some(row) = initializer.declaration {
                    let ty = view.sources[source]
                        .initializers
                        .get(&initializer.signature)
                        .copied()
                        .flatten()
                        .unwrap_or_else(|| {
                            arena.intern(crate::type_checker::core::types::Type::Unknown)
                        });
                    pending.push((row, super::module_type_inputs::Slot::Field, vec![ty]));
                }
            }
        }
        for (row, slot, types) in pending {
            let Some(&owner) = view.canonical.get(&row) else {
                continue;
            };
            super::module_type_inputs::apply(view.info.entry(owner).or_default(), slot, types);
        }
        for (source, signature, declaration, bound) in signatures {
            for (&parameter, &constraint) in bound.generic_parameters.iter().zip(&bound.constraints)
            {
                view.generic_constraints
                    .entry(parameter)
                    .and_modify(|prior| {
                        if *prior != constraint {
                            *prior =
                                Some(arena.intern(crate::type_checker::core::types::Type::Unknown));
                        }
                    })
                    .or_insert(constraint);
            }
            if let Some(owner) = declaration.and_then(|row| view.canonical.get(&row)) {
                let info = view.info.entry(*owner).or_default();
                if info.generic_param_ids.len() == bound.defaults.len() {
                    info.generic_param_default_ids = bound.defaults.clone();
                }
            }
            view.sources
                .get_mut(&source)
                .unwrap()
                .signatures
                .insert(signature, bound);
        }
        generic_groups::install(view);
        for info in view.info.values_mut() {
            super::program_types::finish(info);
        }
        view.bases = super::program_types::bases::Edges::install(bases, &mut view.info, arena);
    }
}

#[cfg(test)]
#[path = "program_view_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "program_compiler_intrinsics_tests.rs"]
mod compiler_intrinsics_tests;

#[cfg(test)]
#[path = "program_conditional_receivers_tests.rs"]
mod conditional_receivers_tests;
