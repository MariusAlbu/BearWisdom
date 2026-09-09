//! Complete signatures are compared structurally; origins remain navigation identities.
use super::*;
use crate::indexer::programs::CallablePolicy;
use crate::type_checker::core::types::Callable;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Ordinary,
    Method,
    StrictCallback,
    BivariantCallback,
}

#[path = "program_callable_context.rs"]
mod context;
#[path = "program_callable_parameters.rs"]
mod parameters;
#[path = "program_callable_values.rs"]
mod values;

impl Eval<'_, '_> {
    pub(in super::super) fn method(
        &mut self,
        source: &Callable<TypeId>,
        target: &Callable<TypeId>,
    ) -> Option<bool> {
        self.relation
            .lookup
            .view
            .callable_policy?
            .bivariant_methods?;
        let source = self.ty(
            self.arena()
                .intern(Type::Callable(Box::new(source.clone()))),
            0,
        )?;
        let target = self.ty(
            self.arena()
                .intern(Type::Callable(Box::new(target.clone()))),
            0,
        )?;
        let (Type::Callable(a), Type::Callable(b)) =
            (self.arena().get(source), self.arena().get(target))
        else {
            return None;
        };
        self.method_value(&a, &b, 0)
    }
    pub(super) fn method_value(
        &mut self,
        source: &Callable<TypeId>,
        target: &Callable<TypeId>,
        depth: usize,
    ) -> Option<bool> {
        self.relation
            .lookup
            .view
            .callable_policy?
            .bivariant_methods?;
        self.compare_callable(source, target, Mode::Method, depth)
    }
    pub(super) fn canonical_callable(
        &mut self,
        callable: &Callable<TypeId>,
        depth: usize,
    ) -> Option<Callable<TypeId>> {
        self.spend(depth)?;
        if !callable.complete {
            return None;
        }
        let mut values = FxHashMap::default();
        for &id in callable.operands() {
            values.insert(id, self.ty(id, depth + 1)?);
        }
        let mut result = callable.map(callable.origin, |id| values[id]);
        // Optional tuple slots describe arity, not merely a union with undefined.
        for (input, output) in callable
            .parameters
            .iter()
            .zip(&mut result.parameters)
            .filter(|(p, _)| p.rest)
        {
            output.ty = self.canonical_rest(input.ty, depth + 1)?;
        }
        let rest_generics: FxHashSet<_> = callable
            .parameters
            .iter()
            .filter(|parameter| parameter.rest)
            .map(|parameter| parameter.ty)
            .collect();
        for (input, output) in callable.generics.iter().zip(&mut result.generics) {
            if !rest_generics.contains(&input.parameter) {
                continue;
            }
            if let Some(constraint) = input.constraint {
                output.constraint = Some(self.canonical_rest(constraint, depth + 1)?);
            }
            if let Some(default) = input.default {
                output.default = Some(self.canonical_rest(default, depth + 1)?);
            }
        }
        Some(result)
    }

    pub(super) fn callable_relation(
        &mut self,
        source: &Callable<TypeId>,
        target: &Callable<TypeId>,
        depth: usize,
    ) -> Option<bool> {
        self.compare_callable(source, target, Mode::Ordinary, depth)
    }

    fn compare_callable(
        &mut self,
        source: &Callable<TypeId>,
        target: &Callable<TypeId>,
        mode: Mode,
        depth: usize,
    ) -> Option<bool> {
        self.spend(depth)?;
        let policy = self.relation.lookup.view.callable_policy?;
        if !source.complete || !target.complete {
            return None;
        }
        self.valid_callable(source, policy, depth + 1)?;
        self.valid_callable(target, policy, depth + 1)?;
        let local = self.callable_generic_ids(target)?;
        let previous = self.rigid.clone();
        self.rigid.extend(local);
        let result = match self.contextual_source(source, target, policy, depth + 1) {
            Some(Ok((source, source_minimum))) => {
                self.signature_relation(&source, target, source_minimum, policy, mode, depth + 1)
            }
            Some(Err(())) => Some(false),
            None => None,
        };
        self.rigid = previous;
        result
    }

    pub(super) fn valid_callable(
        &mut self,
        signature: &Callable<TypeId>,
        policy: CallablePolicy,
        depth: usize,
    ) -> Option<()> {
        let previous = self.subtype;
        self.subtype = false;
        let result = self.valid_callable_inner(signature, policy, depth);
        self.subtype = previous;
        result
    }

    fn valid_callable_inner(
        &mut self,
        signature: &Callable<TypeId>,
        policy: CallablePolicy,
        depth: usize,
    ) -> Option<()> {
        self.spend(depth)?;
        if !signature.complete {
            return None;
        }
        let parameters = self.callable_parameters(signature, depth + 1)?;
        if let Some(rest) = parameters.contextual_rest() {
            let Type::Generic { .. } = self.arena().get(rest) else {
                return None;
            };
            let constraint = signature
                .generics
                .iter()
                .find(|generic| generic.parameter == rest)?
                .constraint?;
            self.valid_contextual_rest_domain(constraint, depth + 1)?;
        }
        for generic in &signature.generics {
            if let (Some(default), Some(constraint)) = (generic.default, generic.constraint) {
                if !self.callable_value(default, constraint, policy, depth + 1)? {
                    return None;
                }
            }
        }
        if let Some(predicate) = &signature.predicate {
            let parameter = signature
                .parameters
                .iter()
                .find(|p| p.declaration == predicate.parameter)?;
            if let Some(asserted) = predicate.asserted {
                // Invalid source is incomplete evidence, including against a
                // broad target which does not inspect its callable signature.
                if !self.callable_value(asserted, parameter.ty, policy, depth + 1)? {
                    return None;
                }
            }
        }
        Some(())
    }

    fn signature_relation(
        &mut self,
        source: &Callable<TypeId>,
        target: &Callable<TypeId>,
        source_minimum: Option<usize>,
        policy: CallablePolicy,
        mode: Mode,
        depth: usize,
    ) -> Option<bool> {
        self.spend(depth)?;
        if !self.signature_parameters(source, target, source_minimum, policy, mode, depth + 1)? {
            return Some(false);
        }
        let source_this = source.parameters.iter().find(|p| p.receiver);
        let target_this = target.parameters.iter().find(|p| p.receiver);
        if let (Some(a), Some(b)) = (source_this, target_this) {
            if a.ty != self.atom(Intrinsic::Void)
                && !self.parameter_relation(
                    a.ty,
                    b.ty,
                    policy,
                    !strict_variance(policy, mode)?,
                    depth + 1,
                )?
            {
                return Some(false);
            }
        }
        if matches!(
            self.arena().get(target.result),
            Type::Intrinsic(Intrinsic::Void | Intrinsic::Any)
        ) {
            return Some(true);
        }
        if let Some(b) = &target.predicate {
            let Some(a) = &source.predicate else {
                return Some(false);
            };
            let slot = |c: &Callable<TypeId>, span| {
                c.parameters.iter().position(|p| p.declaration == span)
            };
            let left = slot(source, a.parameter)?;
            let right = slot(target, b.parameter)?;
            if a.asserts != b.asserts
                || source.parameters[left].receiver != target.parameters[right].receiver
            {
                return Some(false);
            }
            // Receiver slots do not shift the ordinary predicate parameter index.
            let ordinary =
                |c: &Callable<TypeId>, i| c.parameters[..i].iter().filter(|p| !p.receiver).count();
            if !source.parameters[left].receiver
                && ordinary(source, left) != ordinary(target, right)
            {
                return Some(false);
            }
            return match (a.asserted, b.asserted) {
                (Some(a), Some(b)) => self.callable_value(a, b, policy, depth + 1),
                (None, None) => Some(true),
                _ => Some(false),
            };
        }
        let result = self.callable_value(source.result, target.result, policy, depth + 1);
        if mode != Mode::BivariantCallback || result == Some(true) {
            return result;
        }
        either(
            result,
            self.callable_value(target.result, source.result, policy, depth + 1),
        )
    }

    fn callable_parameter(
        &mut self,
        source: TypeId,
        target: TypeId,
        policy: CallablePolicy,
        mode: Mode,
        depth: usize,
    ) -> Option<bool> {
        if matches!(mode, Mode::Ordinary | Mode::Method) {
            if let (Some((a, nullable_a)), Some((b, nullable_b))) = (
                callback_at(self.arena(), source),
                callback_at(self.arena(), target),
            ) {
                if nullable_a == nullable_b && a.predicate.is_none() && b.predicate.is_none() {
                    let mode = if strict_variance(policy, mode)? {
                        Mode::StrictCallback
                    } else {
                        Mode::BivariantCallback
                    };
                    return self.compare_callable(&b, &a, mode, depth + 1);
                }
            }
        }
        self.parameter_relation(
            source,
            target,
            policy,
            matches!(mode, Mode::Ordinary | Mode::Method) && !strict_variance(policy, mode)?,
            depth,
        )
    }

    fn parameter_relation(
        &mut self,
        source: TypeId,
        target: TypeId,
        policy: CallablePolicy,
        bivariant: bool,
        depth: usize,
    ) -> Option<bool> {
        let contra = self.callable_value(target, source, policy, depth + 1);
        if !bivariant || contra == Some(true) {
            return contra;
        }
        let co = self.callable_value(source, target, policy, depth + 1);
        either(contra, co)
    }
}

fn either(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

fn strict_variance(policy: CallablePolicy, mode: Mode) -> Option<bool> {
    Some(match mode {
        Mode::Ordinary => policy.strict_parameters,
        Mode::Method => policy.strict_parameters && !policy.bivariant_methods?,
        Mode::StrictCallback | Mode::BivariantCallback => false,
    })
}

fn callback_at(arena: &TypeArena, ty: TypeId) -> Option<(Box<Callable<TypeId>>, u8)> {
    let mut pending = vec![ty];
    let mut callback = None;
    let mut nullable = 0;
    let mut remaining = 64usize;
    while let Some(ty) = pending.pop() {
        remaining = remaining.checked_sub(1)?;
        match arena.get(ty) {
            Type::Callable(c) if callback.is_none() => callback = Some(c),
            Type::Optional(inner) => {
                nullable |= 1;
                pending.push(inner);
            }
            Type::Intrinsic(Intrinsic::Undefined) => nullable |= 1,
            Type::Intrinsic(Intrinsic::Null) => nullable |= 2,
            Type::Union(parts) => pending.extend(parts),
            _ => return None,
        }
    }
    Some((callback?, nullable))
}

#[cfg(test)]
#[path = "program_callable_relations_tests.rs"]
mod tests;
