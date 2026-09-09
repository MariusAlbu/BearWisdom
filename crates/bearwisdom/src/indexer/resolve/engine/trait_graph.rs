//! Compiled trait identities and source binding overrides. No semantic name search.
use super::{
    contract::{member_applicability::ReceiverPattern, SymbolLookup},
    member_index::MemberNameId,
    module_graph::{BindingResult, ModuleGraph},
    module_trait_inputs::Owner,
    module_type_inputs::Recipe,
};
use crate::indexer::lexical::BindingId;
use crate::type_checker::core::types::{GenericParamId, Type, TypeArena, TypeId};
use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Obligation {
    pub subject: TypeId,
    pub trait_type: TypeId,
}

pub(super) struct Definition {
    pub self_param: GenericParamId,
    pub parameters: Vec<GenericParamId>,
    pub members: FxHashMap<MemberNameId, Vec<i64>>,
    pub enabled: bool,
}
pub(super) struct Implementation {
    pub declaration: i64,
    pub receiver: ReceiverPattern,
    pub trait_type: TypeId,
    pub enabled: bool,
    pub negative: bool,
}
#[derive(Default)]
pub(super) struct Source {
    pub types: FxHashMap<BindingId, TypeId>,
    pub owners: FxHashMap<BindingId, i64>,
}
#[derive(Default)]
pub(super) struct Frame {
    pub parent: Option<usize>,
    pub traits: Vec<i64>,
    pub complete: bool,
}
pub(super) struct QualifiedCall {
    pub caller: Option<i64>,
    pub obligation: Obligation,
}
#[derive(Default)]
pub(super) struct File {
    pub source: Source,
    pub frames: FxHashMap<usize, Frame>,
    pub selectors: FxHashMap<u32, usize>,
    pub qualified_calls: FxHashMap<u32, QualifiedCall>,
}
#[derive(Default)]
pub(super) struct Graph {
    pub definitions: FxHashMap<i64, Definition>,
    pub implementations: FxHashMap<Option<i64>, Vec<Implementation>>,
    pub bounds: FxHashMap<i64, Vec<Obligation>>,
    pub parents: FxHashMap<i64, i64>,
    pub files: FxHashMap<String, File>,
}

pub(super) fn source_type(
    modules: &ModuleGraph,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file: &str,
    source: &Source,
    binding: BindingId,
) -> Option<TypeId> {
    if let Some(&ty) = source.types.get(&binding) {
        return Some(ty);
    }
    let result = modules.binding(file, binding, true);
    if result == BindingResult::Unconfigured {
        return None;
    }
    Some(
        result
            .declaration()
            .and_then(|id| lookup.symbol_by_id(id))
            .map(|symbol| arena.decl(&symbol.qualified_name, lookup.canonical_decl_id(symbol.id)))
            .unwrap_or_else(|| arena.intern(Type::Unknown)),
    )
}

pub(super) fn parameter(
    modules: &ModuleGraph,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file: &str,
    source: &Source,
    binding: BindingId,
    index: usize,
) -> Option<TypeId> {
    let owner = source
        .owners
        .get(&binding)
        .copied()
        .or_else(|| modules.binding(file, binding, true).declaration())?;
    lookup
        .canonical_type_info(lookup.canonical_decl_id(owner))?
        .generic_param_ids
        .get(index)
        .map(|&p| arena.generic_type(p))
}

pub(super) fn materialize(
    modules: &ModuleGraph,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file: &str,
    source: &Source,
    recipe: &Recipe,
) -> TypeId {
    recipe.materialize_with_context(
        arena,
        &|binding| source_type(modules, lookup, arena, file, source, binding),
        &|binding, index| parameter(modules, lookup, arena, file, source, binding, index),
        Some(lookup),
    )
}

impl Graph {
    pub fn build(modules: &ModuleGraph, lookup: &dyn SymbolLookup, arena: &TypeArena) -> Self {
        let mut graph = Self::default();
        for input in modules.inputs.values() {
            let mut file = File::default();
            for header in &input.traits.headers {
                let Some(id) = header
                    .declaration
                    .filter(|id| lookup.symbol_by_id(*id).is_some())
                else {
                    continue;
                };
                file.source
                    .owners
                    .insert(BindingId(header.self_binding), id);
                if let Owner::Declaration(id) = header.owner {
                    let Some(info) = lookup.canonical_type_info(id) else {
                        continue;
                    };
                    let Some(self_param) = info.trait_self_param else {
                        continue;
                    };
                    file.source.types.insert(
                        BindingId(header.self_binding),
                        arena.generic_type(self_param),
                    );
                    let mut members: FxHashMap<_, Vec<_>> = FxHashMap::default();
                    for &member in &header.members {
                        if let Some(name) = lookup
                            .symbol_by_id(member)
                            .and_then(|s| lookup.member_index()?.name(&s.name))
                        {
                            members.entry(name).or_default().push(member);
                        }
                    }
                    graph.definitions.insert(
                        id,
                        Definition {
                            self_param,
                            parameters: info.generic_param_ids.clone(),
                            members,
                            enabled: header.enabled,
                        },
                    );
                }
                for &member in &header.members {
                    graph.parents.insert(member, id);
                }
            }
            // Implementation parameter binders are known before receiver recipes
            // materialize. They are not nominal-owner generic parameter slots.
            for implementation in &input.traits.implementations {
                let ty = materialize(
                    modules,
                    lookup,
                    arena,
                    &input.path,
                    &file.source,
                    &implementation.receiver,
                );
                file.source
                    .types
                    .insert(BindingId(implementation.owner), ty);
            }
            for implementation in &input.traits.implementations {
                let Some(header) = input
                    .traits
                    .headers
                    .iter()
                    .find(|h| h.owner == Owner::Implementation(implementation.owner))
                else {
                    continue;
                };
                let Some(declaration) = header
                    .declaration
                    .filter(|id| lookup.symbol_by_id(*id).is_some())
                else {
                    continue;
                };
                let receiver = file.source.types[&BindingId(implementation.owner)];
                let trait_type = materialize(
                    modules,
                    lookup,
                    arena,
                    &input.path,
                    &file.source,
                    &implementation.trait_type,
                );
                let parameters = lookup
                    .canonical_type_info(declaration)
                    .map(|i| i.generic_param_ids.clone())
                    .unwrap_or_default();
                graph
                    .implementations
                    .entry(super::head_decl::head_decl_id(arena, trait_type))
                    .or_default()
                    .push(Implementation {
                        declaration,
                        receiver: ReceiverPattern {
                            ty: receiver,
                            parameters,
                        },
                        trait_type,
                        enabled: header.enabled,
                        negative: header.negative,
                    });
            }
            for bound in &input.traits.bounds {
                let owner = match bound.owner {
                    Owner::Declaration(id) => Some(id),
                    Owner::Implementation(binding) => {
                        file.source.owners.get(&BindingId(binding)).copied()
                    }
                };
                let Some(owner) = owner.filter(|id| lookup.symbol_by_id(*id).is_some()) else {
                    continue;
                };
                let subject = materialize(
                    modules,
                    lookup,
                    arena,
                    &input.path,
                    &file.source,
                    &bound.subject,
                );
                for recipe in &bound.traits {
                    graph.bounds.entry(owner).or_default().push(Obligation {
                        subject,
                        trait_type: materialize(
                            modules,
                            lookup,
                            arena,
                            &input.path,
                            &file.source,
                            recipe,
                        ),
                    });
                }
            }
            for call in &input.traits.qualified_calls {
                file.qualified_calls.insert(
                    call.selector,
                    QualifiedCall {
                        caller: call.caller,
                        obligation: Obligation {
                            subject: materialize(
                                modules,
                                lookup,
                                arena,
                                &input.path,
                                &file.source,
                                &call.receiver,
                            ),
                            trait_type: materialize(
                                modules,
                                lookup,
                                arena,
                                &input.path,
                                &file.source,
                                &call.trait_type,
                            ),
                        },
                    },
                );
            }
            graph.files.insert(input.path.clone(), file);
        }
        for input in modules.inputs.values() {
            let file = graph.files.get_mut(&input.path).unwrap();
            let providers: FxHashSet<_> = input.traits.providers.iter().copied().collect();
            let partners: FxHashMap<_, _> = input.traits.value_partners.iter().copied().collect();
            for frame in &input.traits.available {
                let mut compiled = Frame {
                    parent: frame.parent,
                    complete: frame.complete,
                    ..Default::default()
                };
                for &binding in &frame.bindings {
                    match modules.binding(&input.path, BindingId(binding), true) {
                        BindingResult::Bound(id) if graph.definitions.contains_key(&id) => {
                            compiled.traits.push(id)
                        }
                        BindingResult::Bound(_) | BindingResult::Namespace(_) => {}
                        BindingResult::Missing
                            if partners.get(&binding).is_some_and(|&value| {
                                modules
                                    .binding(&input.path, BindingId(value), false)
                                    .declaration()
                                    .is_some()
                            }) => {}
                        _ if providers.contains(&binding) => compiled.complete = false,
                        _ => {}
                    }
                }
                compiled.traits.sort_unstable();
                compiled.traits.dedup();
                file.frames.insert(frame.scope, compiled);
            }
            file.selectors
                .extend(input.traits.method_scopes.iter().copied());
        }
        graph
    }

    pub fn assumptions(&self, caller: i64, arena: &TypeArena) -> Result<Vec<Obligation>, ()> {
        let mut pending = Vec::new();
        let mut owner = Some(caller);
        let mut owners = FxHashSet::default();
        while let Some(id) = owner.filter(|id| owners.insert(*id)) {
            pending.extend(self.bounds.get(&id).into_iter().flatten().copied());
            owner = self.parents.get(&id).copied();
        }
        let mut result = Vec::new();
        let mut seen = FxHashSet::default();
        while let Some(bound) = pending.pop() {
            if !seen.insert(bound) {
                continue;
            }
            if seen.len() > 4096 {
                return Err(());
            }
            if let Some(id) = super::head_decl::head_decl_id(arena, bound.trait_type) {
                if let Some(bindings) = self.bindings(id, bound.subject, bound.trait_type, arena) {
                    pending.extend(
                        self.bounds
                            .get(&id)
                            .into_iter()
                            .flatten()
                            .map(|b| substitute(*b, arena, &bindings)),
                    );
                }
            }
            result.push(bound);
        }
        Ok(result)
    }

    pub fn bindings(
        &self,
        id: i64,
        receiver: TypeId,
        applied: TypeId,
        arena: &TypeArena,
    ) -> Option<super::bound_call::Bindings> {
        let definition = self.definitions.get(&id)?;
        let arguments = super::chain::apply_args(arena, applied);
        if arguments.len() != definition.parameters.len() {
            return None;
        }
        if definition
            .parameters
            .iter()
            .zip(&arguments)
            .any(|(&p, &a)| !super::contract::generic_return::argument_kind_agrees(arena, p, a))
        {
            return None;
        }
        let mut bindings: super::bound_call::Bindings = definition
            .parameters
            .iter()
            .copied()
            .zip(arguments)
            .collect();
        bindings.insert(definition.self_param, receiver);
        Some(bindings)
    }
}

pub(super) fn substitute(
    bound: Obligation,
    arena: &TypeArena,
    bindings: &super::bound_call::Bindings,
) -> Obligation {
    let sub = |ty| super::contract::generic_return::substitute(arena, ty, bindings);
    Obligation {
        subject: sub(bound.subject),
        trait_type: sub(bound.trait_type),
    }
}

#[cfg(test)]
#[path = "trait_graph_tests.rs"]
mod tests;
