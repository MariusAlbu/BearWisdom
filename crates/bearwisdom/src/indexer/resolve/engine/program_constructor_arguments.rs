//! Source calls and constructors share full-signature applicability evidence.
use super::merge_proof::types::{ArgumentRelation, Relation};
use super::*;
use crate::indexer::resolve::engine::contract::generic_return;
use crate::type_checker::core::types::{GenericParamId, Intrinsic, LitValue, Type, TypeId};
use source_signatures::Bound;

#[path = "program_member_inference.rs"]
mod members;

pub(super) struct Applied {
    pub result: TypeId,
    pub parameters: Vec<TypeId>,
}

// None: incomplete evidence. Some(None): proved inapplicable.
pub(super) fn applicable(
    relation: &Relation,
    signature: &Bound,
    arguments: &[TypeId],
    types: &[TypeId],
) -> Option<Option<Applied>> {
    contextual(
        relation,
        signature,
        arguments,
        types,
        &|_| false,
        &|_, _| None,
    )
}

pub(super) fn contextual(
    relation: &Relation,
    signature: &Bound,
    arguments: &[TypeId],
    types: &[TypeId],
    deferred: &dyn Fn(usize) -> bool,
    callback: &dyn Fn(usize, TypeId) -> Option<TypeId>,
) -> Option<Option<Applied>> {
    contextual_in_phase(
        relation,
        signature,
        arguments,
        types,
        deferred,
        callback,
        ArgumentRelation::Assignable,
    )
}

pub(super) fn contextual_in_phase(
    relation: &Relation,
    signature: &Bound,
    arguments: &[TypeId],
    types: &[TypeId],
    deferred: &dyn Fn(usize) -> bool,
    callback: &dyn Fn(usize, TypeId) -> Option<TypeId>,
    phase: ArgumentRelation,
) -> Option<Option<Applied>> {
    let arena = relation.arena;
    let params = &signature.generic_parameters;
    // Receiver constraints need call-site receiver proof. They must never
    // eliminate a competitor by being counted as ordinary arguments.
    if signature.syntax.parameters.iter().any(|p| p.receiver) {
        return None;
    }
    if types.len() > params.len() {
        return Some(None);
    }
    if signature.syntax.parameters.len() != signature.parameters.len()
        || signature.defaults.len() != params.len()
        || signature.constraints.len() != params.len()
    {
        return None;
    }
    let mut parameters = Vec::new();
    for (index, (syntax, &ty)) in signature
        .syntax
        .parameters
        .iter()
        .zip(&signature.parameters)
        .enumerate()
    {
        if syntax.rest {
            if index + 1 != signature.parameters.len() {
                return None;
            }
            // Fixed tuple rests retain their real minimum arity. Variadic array
            // rests require selected-source array evidence, never name matching.
            let Type::Tuple(items) = arena.get(ty) else {
                return None;
            };
            for ty in items {
                parameters.push(match arena.get(ty) {
                    Type::Optional(inner) => (inner, true),
                    // Unsupported rest/named tuple syntax can lower to Unknown.
                    // Its arity is unknown too, not a single required slot.
                    Type::Unknown => return None,
                    _ => (ty, false),
                });
            }
        } else {
            parameters.push((ty, syntax.optional));
        }
    }
    let minimum = parameters
        .iter()
        .rposition(|(_, optional)| !optional)
        .map_or(0, |last| last + 1);
    if arguments.len() < minimum || arguments.len() > parameters.len() {
        return Some(None);
    }
    if !types.is_empty()
        && signature
            .defaults
            .iter()
            .enumerate()
            .any(|(i, d)| i >= types.len() && d.is_none())
    {
        return Some(None);
    }
    let mut bindings: FxHashMap<_, _> = params.iter().copied().zip(types.iter().copied()).collect();
    let priorities = std::cell::RefCell::new(FxHashMap::default());
    let active = std::cell::RefCell::new(rustc_hash::FxHashSet::default());
    let inference_site = InferenceSite {
        depth: 0,
        top_level: true,
        priority: 0,
        priorities: &priorities,
        active: &active,
    };
    if types.is_empty() && !params.is_empty() {
        for (index, (&actual, &(expected, _))) in arguments.iter().zip(&parameters).enumerate() {
            if deferred(index) {
                continue;
            }
            // Complete callable shape can disprove a predicate competitor even
            // before its return-only generic parameter has an inference value.
            if matches!(arena.get(expected), Type::Callable(_))
                && relation.argument_with_constraints(
                    actual,
                    expected,
                    params
                        .iter()
                        .copied()
                        .zip(signature.constraints.iter().copied()),
                ) == Some(false)
            {
                return Some(None);
            }
            infer(
                relation,
                signature,
                expected,
                actual,
                &mut bindings,
                inference_site,
            )?;
        }
    }
    for (index, (&actual, &(expected, optional))) in arguments.iter().zip(&parameters).enumerate() {
        if deferred(index) {
            continue;
        }
        let expected = generic_return::substitute(arena, expected, &bindings);
        let expected = if optional {
            arena.intern(Type::Optional(expected))
        } else {
            expected
        };
        if relation.argument_with_constraints(
            actual,
            expected,
            params
                .iter()
                .copied()
                .zip(signature.constraints.iter().copied()),
        ) == Some(false)
        {
            return Some(None);
        }
    }
    let mut arguments = arguments.to_vec();
    for (index, (actual, &(expected, _))) in arguments.iter_mut().zip(&parameters).enumerate() {
        if !deferred(index) {
            continue;
        }
        let context = generic_return::substitute(arena, expected, &bindings);
        *actual = callback(index, context)?;
        if relation.argument_with_constraints(
            *actual,
            expected,
            params
                .iter()
                .copied()
                .zip(signature.constraints.iter().copied()),
        ) == Some(false)
        {
            return Some(None);
        }
        if types.is_empty() && !params.is_empty() {
            infer(
                relation,
                signature,
                expected,
                *actual,
                &mut bindings,
                inference_site,
            )?;
        }
    }
    for (index, &parameter) in params.iter().enumerate() {
        let argument = if let Some(&argument) = bindings.get(&parameter) {
            argument
        } else if let Some(default) = signature.defaults[index] {
            generic_return::substitute(arena, default, &bindings)
        } else if types.is_empty() && arguments.is_empty() {
            signature.constraints[index]
                .map(|ty| generic_return::substitute(arena, ty, &bindings))
                .unwrap_or_else(|| arena.intern(Type::Intrinsic(Intrinsic::Unknown)))
        } else {
            return None;
        };
        let argument = relation.canonical(argument, 0)?;
        if !generic_return::argument_kind_agrees(arena, parameter, argument) {
            return Some(None);
        }
        bindings.insert(parameter, argument);
    }
    for (index, &parameter) in params.iter().enumerate() {
        if let Some(constraint) = signature.constraints[index] {
            if !relation.argument(
                bindings[&parameter],
                generic_return::substitute(arena, constraint, &bindings),
            )? {
                return Some(None);
            }
        }
    }
    // Inference can widen a candidate inferred from ordinary arguments. The
    // callback must be checked under that final context, not an earlier literal
    // parameter type. Re-evaluation is still private and full proof follows.
    for (index, (actual, &(expected, _))) in arguments.iter_mut().zip(&parameters).enumerate() {
        if deferred(index) {
            *actual = callback(
                index,
                generic_return::substitute(arena, expected, &bindings),
            )?;
        }
    }
    for (&actual, &(expected, optional)) in arguments.iter().zip(&parameters) {
        let expected = generic_return::substitute(arena, expected, &bindings);
        let expected = if optional {
            arena.intern(Type::Optional(expected))
        } else {
            expected
        };
        if !relation.argument_in_phase(
            actual,
            expected,
            params.iter().copied().zip(
                signature
                    .constraints
                    .iter()
                    .map(|c| c.map(|ty| generic_return::substitute(arena, ty, &bindings))),
            ),
            phase,
        )? {
            return Some(None);
        }
    }
    let result = relation.canonical(
        generic_return::substitute(arena, signature.result?, &bindings),
        0,
    )?;
    let parameters = parameters
        .into_iter()
        .map(|(ty, _)| generic_return::substitute(arena, ty, &bindings))
        .collect();
    Some(Some(Applied { result, parameters }))
}

#[derive(Clone, Copy)]
struct InferenceSite<'a> {
    depth: usize,
    top_level: bool,
    priority: u8,
    priorities: &'a std::cell::RefCell<FxHashMap<GenericParamId, u8>>,
    active: &'a std::cell::RefCell<rustc_hash::FxHashSet<(TypeId, TypeId)>>,
}
impl InferenceSite<'_> {
    fn nested(self) -> Self {
        Self {
            depth: self.depth + 1,
            top_level: false,
            ..self
        }
    }
    fn union(self) -> Self {
        Self {
            depth: self.depth + 1,
            ..self
        }
    }
}

// Some(true) records an inference occurrence even if its candidate was already
// present. Comparing binding-map mutations loses repeated inference evidence.
fn infer(
    relation: &Relation,
    signature: &Bound,
    expected: TypeId,
    actual: TypeId,
    bindings: &mut FxHashMap<GenericParamId, TypeId>,
    site: InferenceSite<'_>,
) -> Option<bool> {
    let depth = site.depth;
    if depth >= 64 {
        return None;
    }
    let arena = relation.arena;
    let expected = relation.canonical(expected, 0)?;
    let actual = relation.canonical(actual, 0)?;
    if !inference_parameters(arena, expected, &signature.generic_parameters, depth)? {
        return Some(false);
    }
    if let (Some(target), Some(source)) = (
        super::arrays::shape(relation.lookup, arena, expected),
        super::arrays::shape(relation.lookup, arena, actual),
    ) {
        return infer(
            relation,
            signature,
            target.element,
            source.element,
            bindings,
            site.nested(),
        );
    }
    match (arena.get(expected), arena.get(actual)) {
        (Type::Generic { param }, _) if signature.generic_parameters.contains(&param) => {
            // Literal constraints preserve literal candidates; unconstrained
            // top-level value inference widens mutable primitive literals.
            let index = signature
                .generic_parameters
                .iter()
                .position(|&p| p == param)?;
            let direct_result = signature.result == Some(arena.generic_type(param));
            let actual =
                if site.top_level && !direct_result && signature.constraints[index].is_none() {
                    widen(arena, actual)
                } else {
                    actual
                };
            let prior_priority = site.priorities.borrow().get(&param).copied();
            if prior_priority.is_some_and(|prior| prior < site.priority) {
                return Some(true);
            }
            if let Some(&prior) = bindings.get(&param) {
                if prior != actual && prior_priority == Some(site.priority) {
                    // A callback's inferred return widens a primitive literal
                    // candidate from an ordinary argument. Do not invent a
                    // common type for unrelated candidates or constrained IDs.
                    if site.top_level
                        || signature.constraints[index].is_some()
                        || widen(arena, prior) != actual
                    {
                        return None;
                    }
                }
            }
            site.priorities.borrow_mut().insert(param, site.priority);
            bindings.insert(param, actual);
        }
        (Type::Union(targets), _) => {
            return infer_union(relation, signature, targets, actual, bindings, site)
        }
        (_, Type::Union(parts)) => {
            for part in parts {
                infer(relation, signature, expected, part, bindings, site.union())?;
            }
        }
        (Type::Apply { base: a, args: xs }, Type::Apply { base: b, args: ys })
            if a == b && xs.len() == ys.len() =>
        {
            for (x, y) in xs.into_iter().zip(ys) {
                infer(relation, signature, x, y, bindings, site.nested())?;
            }
        }
        (Type::Tuple(xs), Type::Tuple(ys)) if xs.len() == ys.len() => {
            for (x, y) in xs.into_iter().zip(ys) {
                infer(relation, signature, x, y, bindings, site.nested())?;
            }
        }
        (Type::Callable(expected), Type::Callable(actual)) => {
            // Local generic binders need a separate higher-rank inference
            // environment; they must never escape into the method's bindings.
            if !expected.generics.is_empty() || !actual.generics.is_empty() {
                return None;
            }
            for (x, y) in expected.parameters.iter().zip(&actual.parameters) {
                if !inference_parameters(arena, x.ty, &signature.generic_parameters, depth + 1)? {
                    continue;
                }
                if x.receiver != y.receiver || x.rest || y.rest {
                    return None;
                }
                infer(relation, signature, x.ty, y.ty, bindings, site.nested())?;
            }
            if let (Some(x), Some(y)) = (&expected.predicate, &actual.predicate) {
                if let (Some(x), Some(y)) = (x.asserted, y.asserted) {
                    infer(relation, signature, x, y, bindings, site.nested())?;
                }
            }
            infer(
                relation,
                signature,
                expected.result,
                actual.result,
                bindings,
                site.nested(),
            )?;
        }
        // Distinct heads contribute through selected member identities. An
        // unsupported structural position remains incomplete union evidence.
        _ => return members::infer_members(relation, signature, expected, actual, bindings, site),
    }
    Some(true)
}

fn infer_union(
    relation: &Relation,
    signature: &Bound,
    mut targets: Vec<TypeId>,
    actual: TypeId,
    bindings: &mut FxHashMap<GenericParamId, TypeId>,
    site: InferenceSite<'_>,
) -> Option<bool> {
    let arena = relation.arena;
    let mut sources = match arena.get(actual) {
        Type::Union(parts) => parts,
        _ => vec![actual],
    };
    let mut inferred = false;
    // Exact/base-literal matching precedes same-declaration object matching.
    // Remove BOTH matched source and target constituents before fallback.
    for close in [false, true] {
        let mut matched_sources = rustc_hash::FxHashSet::default();
        let mut matched_targets = rustc_hash::FxHashSet::default();
        for &target in &targets {
            for &source in &sources {
                let matched = if close {
                    matches!((arena.get(source), arena.get(target)),
                        (Type::Apply { base: a, .. }, Type::Apply { base: b, .. }) if a == b)
                        || (super::arrays::shape(relation.lookup, arena, source).is_some()
                            && super::arrays::shape(relation.lookup, arena, target).is_some())
                } else {
                    source == target
                        || matches!(
                            (arena.get(source), arena.get(target)),
                            (
                                Type::Literal(LitValue::Str(_) | LitValue::Utf16(_)),
                                Type::Intrinsic(Intrinsic::String)
                            ) | (
                                Type::Literal(LitValue::Int(_) | LitValue::Number(_)),
                                Type::Intrinsic(Intrinsic::Number)
                            )
                        )
                };
                if matched {
                    inferred |= infer(relation, signature, target, source, bindings, site.union())?;
                    matched_sources.insert(source);
                    matched_targets.insert(target);
                }
            }
        }
        sources.retain(|ty| !matched_sources.contains(ty));
        targets.retain(|ty| !matched_targets.contains(ty));
    }
    if targets.is_empty() {
        return Some(inferred);
    }
    if sources.is_empty() {
        // The compiler still infers the ORIGINAL source into unmatched naked
        // targets, but at lower priority than ordinary argument candidates.
        let target = if targets.len() == 1 {
            targets[0]
        } else {
            arena.intern(Type::Union(targets))
        };
        return Some(
            infer(
                relation,
                signature,
                target,
                actual,
                bindings,
                InferenceSite {
                    priority: site.priority | 1,
                    ..site.union()
                },
            )? || inferred,
        );
    }
    let mut naked = Vec::new();
    let mut matched = vec![false; sources.len()];
    for target in targets {
        if matches!(arena.get(target), Type::Generic { param } if signature.generic_parameters.contains(&param))
        {
            naked.push(target);
            continue;
        }
        if !inference_parameters(arena, target, &signature.generic_parameters, site.depth + 1)? {
            // Distinct composite IDs do not establish structural inequality.
            // Until exact structural matching is proved, don't feed a possibly
            // matched composite source into a naked fallback parameter.
            if composite(arena, target) && sources.iter().any(|&source| composite(arena, source)) {
                return None;
            }
            continue;
        }
        for (index, &source) in sources.iter().enumerate() {
            matched[index] |= infer(relation, signature, target, source, bindings, site.union())?;
        }
    }
    if naked.len() > 1 {
        return None;
    }
    inferred |= matched.iter().any(|&value| value);
    if let Some(&target) = naked.first() {
        let rest: Vec<_> = sources
            .into_iter()
            .zip(matched)
            .filter_map(|(ty, matched)| (!matched).then_some(ty))
            .collect();
        if !rest.is_empty() {
            let actual = if rest.len() == 1 {
                rest[0]
            } else {
                arena.intern(Type::Union(rest))
            };
            inferred |= infer(relation, signature, target, actual, bindings, site.union())?;
        }
    }
    Some(inferred)
}

fn composite(arena: &TypeArena, ty: TypeId) -> bool {
    matches!(
        arena.get(ty),
        Type::Decl { .. }
            | Type::Apply { .. }
            | Type::Callable(_)
            | Type::Function { .. }
            | Type::Tuple(_)
            | Type::Operator(_)
            | Type::Constructor(_)
            | Type::Intersection(_)
    )
}

fn inference_parameters(
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
        Type::Union(items) | Type::Intersection(items) | Type::Tuple(items) => items,
        Type::Optional(inner) | Type::Constructor(inner) => vec![inner],
        Type::Callable(c) => c.operands().copied().collect(),
        Type::Object(object) => object.operands().copied().collect(),
        Type::Function {
            mut params,
            return_,
        } => {
            params.push(return_);
            params
        }
        Type::Operator(op) => op.operands().copied().collect(),
        Type::Unknown => return None,
        _ => return Some(false),
    };
    let mut found = false;
    for child in children {
        found |= inference_parameters(arena, child, parameters, depth + 1)?;
    }
    Some(found)
}

fn widen(arena: &TypeArena, ty: TypeId) -> TypeId {
    let kind = match arena.get(ty) {
        Type::Literal(LitValue::Str(_) | LitValue::Utf16(_)) => Intrinsic::String,
        Type::Literal(LitValue::Int(_) | LitValue::Number(_)) => Intrinsic::Number,
        Type::Literal(LitValue::Bool(_)) => Intrinsic::Boolean,
        Type::Literal(LitValue::BigInt { .. }) => Intrinsic::BigInt,
        _ => return ty,
    };
    arena.intern(Type::Intrinsic(kind))
}

#[cfg(test)]
#[path = "program_constructor_arguments_tests.rs"]
mod tests;
