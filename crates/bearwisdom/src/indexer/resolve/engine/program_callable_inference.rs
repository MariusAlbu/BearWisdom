//! Recursive candidate inference and constraint validation for contextual signatures.
use super::*;
use crate::type_checker::core::types::GenericParamId;

impl Eval<'_, '_> {
    pub(super) fn contextual_rest_constraint(
        &mut self,
        candidate: TypeId,
        constraint: TypeId,
        policy: CallablePolicy,
        depth: usize,
    ) -> Option<bool> {
        self.spend(depth)?;
        let candidate = self.canonical_rest(candidate, depth + 1)?;
        let constraint = self.canonical_rest(constraint, depth + 1)?;
        if let Type::Union(parts) = self.arena().get(candidate) {
            let mut unknown = false;
            for part in parts {
                match self.contextual_rest_constraint(part, constraint, policy, depth + 1) {
                    Some(false) => return Some(false),
                    None => unknown = true,
                    _ => {}
                }
            }
            return if unknown { None } else { Some(true) };
        }
        if let Type::Union(parts) = self.arena().get(constraint) {
            let mut unknown = false;
            for part in parts {
                match self.contextual_rest_constraint(candidate, part, policy, depth + 1) {
                    Some(true) => return Some(true),
                    None => unknown = true,
                    _ => {}
                }
            }
            return if unknown { None } else { Some(false) };
        }
        if let Type::Operator(TypeOperator::Readonly(inner)) = self.arena().get(candidate) {
            return self.contextual_rest_constraint(inner, constraint, policy, depth + 1);
        }
        if let Type::Operator(TypeOperator::Readonly(inner)) = self.arena().get(constraint) {
            return self.contextual_rest_constraint(candidate, inner, policy, depth + 1);
        }
        if let (Type::Tuple(candidate), Type::Tuple(constraint)) =
            (self.arena().get(candidate), self.arena().get(constraint))
        {
            let minimum = |items: &[TypeId]| {
                items
                    .iter()
                    .rposition(|&ty| !matches!(self.arena().get(ty), Type::Optional(_)))
                    .map_or(0, |i| i + 1)
            };
            if minimum(&candidate) < minimum(&constraint) || candidate.len() > constraint.len() {
                return Some(false);
            }
            for (candidate, constraint) in candidate.into_iter().zip(constraint) {
                if !self.contextual_constraint(candidate, constraint, policy, depth + 1)? {
                    return Some(false);
                }
            }
            return Some(true);
        }
        if let Type::Tuple(items) = self.arena().get(candidate) {
            if let Some(array) = arrays::shape(self.relation.lookup, self.arena(), constraint) {
                for item in items {
                    if !self.contextual_constraint(item, array.element, policy, depth + 1)? {
                        return Some(false);
                    }
                }
                return Some(true);
            }
        }
        if matches!(self.arena().get(constraint), Type::Tuple(_)) {
            return Some(false);
        }
        self.contextual_constraint(candidate, constraint, policy, depth + 1)
    }

    pub(super) fn contextual_constraint(
        &mut self,
        candidate: TypeId,
        constraint: TypeId,
        policy: CallablePolicy,
        depth: usize,
    ) -> Option<bool> {
        self.spend(depth)?;
        if candidate == constraint
            || self
                .obligation(candidate, constraint, &mut FxHashSet::default(), depth + 1)
                .is_some()
        {
            return Some(true);
        }
        if let Type::Generic { param } = self.arena().get(candidate) {
            if self.rigid.contains(&param) {
                let Some(domain) = self.constraint(param)? else {
                    return Some(matches!(
                        self.arena().get(constraint),
                        Type::Intrinsic(Intrinsic::Any | Intrinsic::Unknown)
                    ));
                };
                return self.callable_value(domain, constraint, policy, depth + 1);
            }
        }
        self.callable_value(candidate, constraint, policy, depth + 1)
    }

    pub(super) fn infer_contextual(
        &mut self,
        pattern: TypeId,
        actual: TypeId,
        open: &FxHashSet<GenericParamId>,
        bindings: &mut FxHashMap<GenericParamId, TypeId>,
        policy: CallablePolicy,
        depth: usize,
    ) -> Option<()> {
        self.spend(depth)?;
        let pattern = self.ty(pattern, depth + 1)?;
        let actual = self.ty(actual, depth + 1)?;
        if let Type::Generic { param } = self.arena().get(pattern) {
            if open.contains(&param) {
                if let Some(&prior) = bindings.get(&param) {
                    if prior == actual {
                        return Some(());
                    }
                    let prior_to_actual = self.callable_value(prior, actual, policy, depth + 1);
                    let actual_to_prior = self.callable_value(actual, prior, policy, depth + 1);
                    match (prior_to_actual, actual_to_prior) {
                        (Some(true), Some(false)) => {
                            bindings.insert(param, actual);
                            return Some(());
                        }
                        (_, Some(true)) => return Some(()),
                        _ => return None,
                    }
                }
                bindings.insert(param, actual);
                return Some(());
            }
        }
        if !self.needs_contextual_inference(pattern, open, bindings, depth + 1)? {
            return Some(());
        }
        if let (Some(pattern), Some(actual)) = (
            arrays::shape(self.relation.lookup, self.arena(), pattern),
            arrays::shape(self.relation.lookup, self.arena(), actual),
        ) {
            return self.infer_contextual(
                pattern.element,
                actual.element,
                open,
                bindings,
                policy,
                depth + 1,
            );
        }
        match (self.arena().get(pattern), self.arena().get(actual)) {
            (
                Type::Apply {
                    base: pattern_base,
                    args: pattern_args,
                },
                Type::Apply {
                    base: actual_base,
                    args: actual_args,
                },
            ) if pattern_base == actual_base && pattern_args.len() == actual_args.len() => {
                for (pattern, actual) in pattern_args.into_iter().zip(actual_args) {
                    self.infer_contextual(pattern, actual, open, bindings, policy, depth + 1)?;
                }
            }
            (Type::Tuple(patterns), Type::Tuple(actuals)) if patterns.len() == actuals.len() => {
                for (pattern, actual) in patterns.into_iter().zip(actuals) {
                    self.infer_contextual(pattern, actual, open, bindings, policy, depth + 1)?;
                }
            }
            (Type::Optional(pattern), Type::Optional(actual))
            | (Type::AsyncWrapper(pattern), Type::AsyncWrapper(actual))
            | (Type::Iterator(pattern), Type::Iterator(actual))
            | (Type::Constructor(pattern), Type::Constructor(actual)) => {
                self.infer_contextual(pattern, actual, open, bindings, policy, depth + 1)?
            }
            (
                Type::Function {
                    params: patterns,
                    return_: pattern_result,
                },
                Type::Function {
                    params: actuals,
                    return_: actual_result,
                },
            ) if patterns.len() == actuals.len() => {
                for (pattern, actual) in patterns.into_iter().zip(actuals) {
                    self.infer_contextual(pattern, actual, open, bindings, policy, depth + 1)?;
                }
                self.infer_contextual(
                    pattern_result,
                    actual_result,
                    open,
                    bindings,
                    policy,
                    depth + 1,
                )?;
            }
            _ => return None,
        }
        Some(())
    }

    pub(super) fn needs_contextual_inference(
        &mut self,
        ty: TypeId,
        open: &FxHashSet<GenericParamId>,
        bindings: &FxHashMap<GenericParamId, TypeId>,
        depth: usize,
    ) -> Option<bool> {
        self.spend(depth)?;
        let mut children = match self.arena().get(ty) {
            Type::Generic { param } => {
                return Some(open.contains(&param) && !bindings.contains_key(&param));
            }
            Type::Apply { base, mut args } => {
                args.push(base);
                args
            }
            Type::Function {
                mut params,
                return_,
            } => {
                params.push(return_);
                params
            }
            Type::Callable(callable) => callable.operands().copied().collect(),
            Type::Object(object) => object.operands().copied().collect(),
            Type::Tuple(items) | Type::Union(items) | Type::Intersection(items) => items,
            Type::Operator(operator) => operator.operands().copied().collect(),
            Type::Optional(inner)
            | Type::AsyncWrapper(inner)
            | Type::Iterator(inner)
            | Type::Constructor(inner) => vec![inner],
            Type::Indirect { inner, .. } => vec![inner],
            _ => vec![],
        };
        while let Some(child) = children.pop() {
            if self.needs_contextual_inference(child, open, bindings, depth + 1)? {
                return Some(true);
            }
        }
        Some(false)
    }
}
