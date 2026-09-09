//! Generic source signatures are instantiated from the contextual target shape.
use super::*;
use crate::type_checker::core::types::GenericParamId;

#[path = "program_callable_inference.rs"]
mod inference;

impl Eval<'_, '_> {
    pub(super) fn valid_contextual_rest_domain(
        &mut self,
        domain: TypeId,
        depth: usize,
    ) -> Option<()> {
        self.spend(depth)?;
        let domain = self.ty(domain, depth + 1)?;
        if arrays::shape(self.relation.lookup, self.arena(), domain).is_some() {
            return Some(());
        }
        match self.arena().get(domain) {
            Type::Tuple(_) => Some(()),
            Type::Union(parts) => {
                for part in parts {
                    self.valid_contextual_rest_domain(part, depth + 1)?;
                }
                Some(())
            }
            Type::Operator(TypeOperator::Readonly(inner)) => {
                self.valid_contextual_rest_domain(inner, depth + 1)
            }
            Type::Intrinsic(Intrinsic::Any) => Some(()),
            _ => None,
        }
    }

    pub(super) fn callable_generic_ids(
        &self,
        signature: &Callable<TypeId>,
    ) -> Option<FxHashSet<GenericParamId>> {
        let mut result = FxHashSet::default();
        for generic in &signature.generics {
            let Type::Generic { param } = self.arena().get(generic.parameter) else {
                return None;
            };
            if !result.insert(param) {
                return None;
            }
        }
        Some(result)
    }

    pub(super) fn contextual_source(
        &mut self,
        source: &Callable<TypeId>,
        target: &Callable<TypeId>,
        policy: CallablePolicy,
        depth: usize,
    ) -> Option<Result<(Callable<TypeId>, Option<usize>), ()>> {
        self.spend(depth)?;
        if source.generics.is_empty() {
            return Some(Ok((source.clone(), None)));
        }
        let open = self.callable_generic_ids(source)?;
        let source_parameters = self.callable_parameters(source, depth + 1)?;
        let target_parameters = self.callable_parameters(target, depth + 1)?;
        let mut bindings = FxHashMap::default();
        let contextual_rest = source_parameters.contextual_rest().and_then(|ty| {
            let Type::Generic { param } = self.arena().get(ty) else {
                return None;
            };
            open.contains(&param).then_some((ty, param))
        });
        if source_parameters.contextual_rest().is_some() && contextual_rest.is_none() {
            return None;
        }
        let source_minimum = contextual_rest.map(|_| source_parameters.minimum());
        if let Some((pattern, param)) = contextual_rest {
            let constraint = source
                .generics
                .iter()
                .find(|generic| generic.parameter == pattern)?
                .constraint?;
            let constraint = self.canonical_rest(constraint, depth + 1)?;
            // A direct generic rest is valid only when its declared domain is
            // a compiler-recognized array or finite tuple domain. The
            // contextual tail can then infer a finite tuple or the target's
            // original array rest.
            self.valid_contextual_rest_domain(constraint, depth + 1)?;
            let prefix = source_parameters.fixed_count();
            for index in 0..prefix {
                let (Some((pattern, _)), Some((actual, _))) = (
                    source_parameters.parameter(index),
                    target_parameters.parameter(index),
                ) else {
                    continue;
                };
                self.infer_contextual(pattern, actual, &open, &mut bindings, policy, depth + 1)?;
            }
            let actual = self.contextual_tail(&target_parameters, prefix, depth + 1)?;
            self.infer_contextual(pattern, actual, &open, &mut bindings, policy, depth + 1)?;
            debug_assert!(bindings.contains_key(&param));
        } else {
            if source_parameters.has_finite()
                || target_parameters.has_finite()
                || target_parameters.contextual_rest().is_some()
            {
                return None;
            }
            let count = source_parameters.count().max(target_parameters.count());
            for index in 0..count {
                let (Some((pattern, _)), Some((actual, _))) = (
                    source_parameters.parameter(index),
                    target_parameters.parameter(index),
                ) else {
                    continue;
                };
                self.infer_contextual(pattern, actual, &open, &mut bindings, policy, depth + 1)?;
            }
        }
        if let (Some(pattern), Some(actual)) = (
            source.parameters.iter().find(|p| p.receiver),
            target.parameters.iter().find(|p| p.receiver),
        ) {
            self.infer_contextual(
                pattern.ty,
                actual.ty,
                &open,
                &mut bindings,
                policy,
                depth + 1,
            )?;
        }
        match (&source.predicate, &target.predicate) {
            (Some(pattern), Some(actual))
                if pattern.asserts == actual.asserts
                    && pattern.asserted.is_some()
                    && actual.asserted.is_some() =>
            {
                self.infer_contextual(
                    pattern.asserted?,
                    actual.asserted?,
                    &open,
                    &mut bindings,
                    policy,
                    depth + 1,
                )?;
            }
            _ => {
                let mut inferred = false;
                if let Some((pattern, param)) = contextual_rest {
                    if source.result == pattern {
                        let candidate = *bindings.get(&param)?;
                        let constraint = source
                            .generics
                            .iter()
                            .find(|generic| generic.parameter == pattern)?
                            .constraint?;
                        let constraint = self.canonical_rest(constraint, depth + 1)?;
                        let actual = self.canonical_rest(target.result, depth + 1)?;
                        if !self.contextual_rest_constraint(
                            candidate,
                            constraint,
                            policy,
                            depth + 1,
                        )? && self.contextual_rest_constraint(
                            actual,
                            constraint,
                            policy,
                            depth + 1,
                        )? {
                            bindings.insert(param, actual);
                            inferred = true;
                        }
                    }
                }
                if !inferred {
                    self.infer_contextual(
                        source.result,
                        target.result,
                        &open,
                        &mut bindings,
                        policy,
                        depth + 1,
                    )?;
                }
            }
        }
        for generic in &source.generics {
            let Type::Generic { param } = self.arena().get(generic.parameter) else {
                return None;
            };
            let Some(&candidate) = bindings.get(&param) else {
                continue;
            };
            if !crate::indexer::resolve::engine::contract::generic_return::argument_kind_agrees(
                self.arena(),
                param,
                candidate,
            ) {
                return Some(Err(()));
            }
            if let Some(constraint) = generic.constraint {
                let constraint = substitute(self.arena(), constraint, &bindings);
                if self.needs_contextual_inference(constraint, &open, &bindings, depth + 1)? {
                    return None;
                }
                let constraint = self.canonical_rest(constraint, depth + 1)?;
                let accepted = if contextual_rest.is_some_and(|(_, rest)| rest == param) {
                    self.contextual_rest_constraint(candidate, constraint, policy, depth + 1)?
                } else {
                    self.contextual_constraint(candidate, constraint, policy, depth + 1)?
                };
                if !accepted {
                    if contextual_rest.is_some_and(|(_, rest)| rest == param) {
                        // TypeScript falls back to the rest constraint when a
                        // contextual tail candidate lies outside that domain,
                        // then performs the ordinary signature comparison.
                        bindings.insert(param, constraint);
                    } else {
                        return Some(Err(()));
                    }
                }
            }
        }
        for parameter in &source.parameters {
            if self.needs_contextual_inference(parameter.ty, &open, &bindings, depth + 1)? {
                return None;
            }
        }
        if self.needs_contextual_inference(source.result, &open, &bindings, depth + 1)? {
            return None;
        }
        if let Some(asserted) = source
            .predicate
            .as_ref()
            .and_then(|predicate| predicate.asserted)
        {
            if self.needs_contextual_inference(asserted, &open, &bindings, depth + 1)? {
                return None;
            }
        }
        let mut instantiated =
            source.map(source.origin, |&ty| substitute(self.arena(), ty, &bindings));
        instantiated.generics.clear();
        Some(Ok((instantiated, source_minimum)))
    }
}
