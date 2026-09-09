//! Generic declaration evidence is reconciled privately, never last-row-wins.
use super::merge_proof::types::Relation;
use super::*;
use crate::type_checker::core::types::{GenericParamId, Type, TypeId};
use rustc_hash::FxHashSet;
use source_signatures::{Bound, SignatureId};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ParameterName(u32);

struct Part {
    source: SourceInstanceId,
    signature: Option<SignatureId>,
    names: Option<Vec<ParameterName>>,
    complete: bool,
}

#[derive(Default)]
pub(super) struct Index(FxHashMap<i64, Vec<Part>>);

pub(super) struct Group {
    pub bound: Bound,
    parts: Vec<(Option<Vec<ParameterName>>, Option<Bound>, bool)>,
}

impl Index {
    pub(super) fn capture(view: &View, inputs: &computed_keys::Sources) -> Self {
        // Selected-view ingestion: raw source spellings end in this local pool.
        let mut names = FxHashMap::default();
        let mut groups = FxHashMap::default();
        for (_, &source, input) in inputs {
            let spellings: FxHashMap<_, _> =
                input.names.iter().map(|(id, name)| (*id, name)).collect();
            let signatures: FxHashMap<_, _> =
                input.source_signatures.iter().map(|s| (s.id, s)).collect();
            for interface in &input.interfaces {
                let Some(&owner) = view.canonical.get(&interface.owner) else {
                    continue;
                };
                let signature = interface
                    .generic_signature
                    .and_then(|id| signatures.get(&id).copied());
                let parameter_names = signature
                    .map(|s| {
                        s.generics
                            .iter()
                            .map(|parameter| {
                                let spelling = *spellings.get(&parameter.name?)?;
                                let next = ParameterName(names.len() as u32);
                                Some(*names.entry(spelling).or_insert(next))
                            })
                            .collect::<Option<Vec<_>>>()
                    })
                    .unwrap_or_else(|| interface.plain_parameters.then(Vec::new));
                let part = Part {
                    source,
                    signature: interface.generic_signature,
                    names: parameter_names,
                    complete: signature.map_or(interface.plain_parameters, |s| {
                        s.syntax.type_parameters_complete
                    }),
                };
                groups.entry(owner).or_insert_with(Vec::new).push(part);
            }
        }
        Self(groups)
    }

    pub(super) fn materialize(&self, view: &View) -> FxHashMap<i64, Group> {
        self.0
            .iter()
            .filter_map(|(&owner, inputs)| {
                let parameters = view.info.get(&owner)?.generic_param_ids.clone();
                let parts: Vec<_> = inputs
                    .iter()
                    .map(|part| {
                        (
                            part.names.clone(),
                            match part.signature {
                                Some(id) => view
                                    .sources
                                    .get(&part.source)
                                    .and_then(|s| s.signatures.get(&id))
                                    .cloned(),
                                None => Some(Bound::default()),
                            },
                            part.complete,
                        )
                    })
                    .collect();
                let mut bound = Bound {
                    constraints: vec![None; parameters.len()],
                    defaults: vec![None; parameters.len()],
                    generic_parameters: parameters,
                    ..Default::default()
                };
                // A supplied candidate is provisional until all declarations agree.
                // Preserve every raw signature below for the subsequent proof.
                for (_, part, _) in &parts {
                    if let Some(part) = part {
                        for (target, source) in bound.constraints.iter_mut().zip(&part.constraints)
                        {
                            if target.is_none() {
                                *target = *source;
                            }
                        }
                        for (target, source) in bound.defaults.iter_mut().zip(&part.defaults) {
                            if target.is_none() {
                                *target = *source;
                            }
                        }
                    }
                }
                Some((owner, Group { bound, parts }))
            })
            .collect()
    }
}

pub(super) fn install(view: &mut View) {
    view.generic_declarations = view.generic_inputs.materialize(view);
    for (&owner, group) in &view.generic_declarations {
        if let Some(info) = view.info.get_mut(&owner) {
            info.generic_param_default_ids = group.bound.defaults.clone();
        }
        for (&parameter, &constraint) in group
            .bound
            .generic_parameters
            .iter()
            .zip(&group.bound.constraints)
        {
            view.generic_constraints.insert(parameter, constraint);
        }
    }
}

pub(super) fn rejected(view: &View, tree: &Compilation, arena: &TypeArena) -> FxHashSet<i64> {
    let lookup = Lookup {
        tree,
        view,
        source: None,
    };
    let relation = Relation {
        lookup: &lookup,
        arena,
    };
    view.generic_declarations
        .iter()
        .filter(|(_, group)| group.parts.len() > 1)
        .filter_map(|(&owner, group)| (!group.valid(&relation)).then_some(owner))
        .collect()
}

impl Group {
    fn valid(&self, relation: &Relation) -> bool {
        self.prove(relation).is_some()
    }

    fn prove(&self, relation: &Relation) -> Option<()> {
        let parameters = &self.bound.generic_parameters;
        let longest = self
            .parts
            .iter()
            .filter_map(|(names, _, _)| names.as_ref())
            .max_by_key(|n| n.len())?;
        if longest.len() != parameters.len()
            || longest.iter().collect::<FxHashSet<_>>().len() != longest.len()
        {
            return None;
        }
        let minimum = self
            .bound
            .defaults
            .iter()
            .rposition(Option::is_none)
            .map_or(0, |index| index + 1);
        if self.bound.defaults[..minimum].iter().any(Option::is_some) {
            return None;
        }
        for (names, part, complete) in &self.parts {
            let names = names.as_ref()?;
            let part = part.as_ref()?;
            if !complete
                || !longest.starts_with(names)
                || names.len() < minimum
                || part.generic_parameters != parameters[..names.len()]
                || part.constraints.len() != names.len()
                || part.defaults.len() != names.len()
            {
                return None;
            }
            // A later augmentation cannot repair invalid syntax in an earlier
            // declaration (a required parameter after a supplied default).
            let required = part
                .defaults
                .iter()
                .rposition(Option::is_none)
                .map_or(0, |index| index + 1);
            if part.defaults[..required].iter().any(Option::is_some) {
                return None;
            }
            for (index, (&constraint, &default)) in
                part.constraints.iter().zip(&part.defaults).enumerate()
            {
                for (source, effective) in [
                    (constraint, self.bound.constraints[index]),
                    (default, self.bound.defaults[index]),
                ] {
                    if let Some(source) = source {
                        if !relation.equal(source, effective?) {
                            return None;
                        }
                    }
                }
                if let Some(default) = default {
                    if references(relation.arena, default, &parameters[index..], 0)? {
                        return None;
                    }
                }
            }
        }
        for (index, &parameter) in parameters.iter().enumerate() {
            no_constraint_cycle(relation, parameter, &mut FxHashSet::default(), 0)?;
            if let Some(default) = self.bound.defaults[index] {
                let default = relation.canonical(default, 0)?;
                if let Some(constraint) = self.bound.constraints[index] {
                    if relation.argument(default, constraint) != Some(true) {
                        return None;
                    }
                }
            }
        }
        Some(())
    }
}

fn references(
    arena: &TypeArena,
    ty: TypeId,
    parameters: &[GenericParamId],
    depth: usize,
) -> Option<bool> {
    if depth >= 64 {
        return None;
    }
    let children = match arena.get(ty) {
        Type::Generic { param } => return Some(parameters.contains(&param)),
        Type::Apply { base, mut args } => {
            args.push(base);
            args
        }
        Type::Union(parts) | Type::Intersection(parts) | Type::Tuple(parts) => parts,
        Type::Optional(inner) | Type::Constructor(inner) => vec![inner],
        Type::Operator(op) => op.operands().copied().collect(),
        Type::Callable(c) => c.operands().copied().collect(),
        Type::Object(object) => object.operands().copied().collect(),
        Type::Function {
            mut params,
            return_,
        } => {
            params.push(return_);
            params
        }
        Type::Unknown => return None,
        _ => return Some(false),
    };
    for child in children {
        if references(arena, child, parameters, depth + 1)? {
            return Some(true);
        }
    }
    Some(false)
}

fn no_constraint_cycle(
    relation: &Relation,
    parameter: GenericParamId,
    seen: &mut FxHashSet<GenericParamId>,
    depth: usize,
) -> Option<()> {
    if depth >= 64 || !seen.insert(parameter) {
        return None;
    }
    if let Some(constraint) = relation.lookup.generic_constraint(parameter)? {
        let constraint = relation.canonical(constraint, 0)?;
        direct_constraint(relation, constraint, seen, depth + 1)?;
    }
    seen.remove(&parameter);
    Some(())
}

fn direct_constraint(
    relation: &Relation,
    ty: TypeId,
    seen: &mut FxHashSet<GenericParamId>,
    depth: usize,
) -> Option<()> {
    if depth >= 64 {
        return None;
    }
    match relation.arena.get(ty) {
        Type::Generic { param } => no_constraint_cycle(relation, param, seen, depth + 1)?,
        Type::Union(parts) | Type::Intersection(parts) => {
            for part in parts {
                direct_constraint(relation, part, seen, depth + 1)?;
            }
        }
        _ => {}
    }
    Some(())
}

#[cfg(test)]
#[path = "program_generic_groups_tests.rs"]
mod tests;
