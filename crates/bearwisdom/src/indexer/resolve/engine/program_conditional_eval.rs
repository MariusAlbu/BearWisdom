//! Conditional branch selection requires positive or negative evidence, not a
//! failed conservative assignment query. Pattern binders carry source IDs.
use super::*;

impl Eval<'_, '_> {
    pub(in super::super) fn argument(&mut self, from: TypeId, to: TypeId) -> Option<bool> {
        if self.subtype {
            return self.subtype_value(from, to, 0);
        }
        let from = self.ty(from, 0)?;
        let to = self.ty(to, 0)?;
        if self.relation.lookup.view.callable_policy.is_some() {
            return self.callable_value(from, to, self.relation.lookup.view.callable_policy?, 0);
        }
        self.proof_value(from, 0)?;
        self.proof_value(to, 0)?;
        if self
            .obligation(from, to, &mut FxHashSet::default(), 0)
            .is_some()
        {
            return Some(true);
        }
        self.matches_pattern(from, to, &mut FxHashMap::default(), 0)
    }
    pub(super) fn conditional(
        &mut self,
        check: TypeId,
        pattern: TypeId,
        yes: TypeId,
        no: TypeId,
        distributive: Option<bool>,
        depth: usize,
    ) -> Option<TypeId> {
        self.spend(depth)?;
        let check = self.ty(check, depth + 1)?;
        if let Some(owner) =
            crate::indexer::resolve::engine::head_decl::head_decl_id(self.arena(), check)
        {
            self.nominal_bindings(check, owner, depth + 1)?;
        }
        // Union/any distribution needs the original checked binder, not a
        // rewrite of every coincidentally equal TypeId in the branch.
        if matches!(
            self.arena().get(check),
            Type::Generic { .. } | Type::Union(_) | Type::Intrinsic(Intrinsic::Any)
        ) {
            return None;
        }
        if distributive != Some(false) && check == self.atom(Intrinsic::Boolean) {
            return None;
        }
        self.pattern(pattern, depth + 1)?;
        if check == self.atom(Intrinsic::Never) {
            return if distributive == Some(true) {
                Some(check)
            } else {
                None
            };
        }
        let mut inferred = FxHashMap::default();
        let matched = self.matches_pattern(check, pattern, &mut inferred, depth + 1)?;
        self.ty(
            if matched {
                substitute(self.arena(), yes, &inferred)
            } else {
                no
            },
            depth + 1,
        )
    }

    fn pattern(&mut self, pattern: TypeId, depth: usize) -> Option<()> {
        self.spend(depth)?;
        match self.arena().get(pattern) {
            Type::Operator(TypeOperator::Infer(parameter)) => {
                let Type::Generic { param } = self.arena().get(parameter) else {
                    return None;
                };
                if let Some(constraint) = self.constraint(param)? {
                    self.ty(constraint, depth + 1)?;
                }
            }
            Type::Operator(TypeOperator::Object(properties)) => {
                let mut keys = FxHashSet::default();
                for p in properties {
                    let key = self.ty(p.key, depth + 1)?;
                    if p.index
                        || !keys.insert(key)
                        || !matches!(
                            self.arena().get(key),
                            Type::Literal(LitValue::Str(_) | LitValue::Utf16(_))
                                | Type::UniqueSymbol(_)
                        )
                    {
                        return None;
                    }
                    self.pattern(p.value, depth + 1)?;
                }
            }
            // Other complete types can be positive assignment evidence. An
            // unsupported infer position cannot canonicalize and fails here.
            _ => {
                let pattern = self.ty(pattern, depth + 1)?;
                if let Some(owner) =
                    crate::indexer::resolve::engine::head_decl::head_decl_id(self.arena(), pattern)
                {
                    self.nominal_bindings(pattern, owner, depth + 1)?;
                }
            }
        }
        Some(())
    }

    pub(super) fn matches_pattern(
        &mut self,
        check: TypeId,
        pattern: TypeId,
        inferred: &mut FxHashMap<GenericParamId, TypeId>,
        depth: usize,
    ) -> Option<bool> {
        self.spend(depth)?;
        if let Type::Operator(TypeOperator::Infer(parameter)) = self.arena().get(pattern) {
            self.proof_value(check, depth + 1)?;
            let Type::Generic { param } = self.arena().get(parameter) else {
                return None;
            };
            if let Some(constraint) = self.constraint(param)? {
                let constraint = self.ty(constraint, depth + 1)?;
                if !self.matches_pattern(check, constraint, inferred, depth + 1)? {
                    return Some(false);
                }
            }
            // Duplicate source binders are not merged by spelling. A future
            // variance-aware candidate collector must prove that combination.
            if inferred
                .insert(param, check)
                .is_some_and(|prior| prior != check)
            {
                return None;
            }
            return Some(true);
        }
        if let Type::Operator(TypeOperator::Object(properties)) = self.arena().get(pattern) {
            if properties.is_empty() {
                return if self.relation.assign(check, pattern, depth + 1) {
                    Some(true)
                } else {
                    None
                };
            }
            let source = self.key_properties(check, depth + 1)?;
            if !source.is_empty()
                && properties.iter().all(|p| p.optional)
                && !source
                    .iter()
                    .any(|a| properties.iter().any(|b| a.key == b.key))
            {
                return Some(false);
            }
            for target in properties {
                self.spend(depth)?;
                let key = self.ty(target.key, depth + 1)?;
                let selected: Vec<_> = source.iter().filter(|p| !p.index && p.key == key).collect();
                let Some(property) = selected.first() else {
                    if !target.optional {
                        return Some(false);
                    }
                    // Absence does not supply an inference candidate.
                    if !self.matches_pattern(
                        self.atom(Intrinsic::Unknown),
                        target.value,
                        inferred,
                        depth + 1,
                    )? {
                        return None;
                    }
                    continue;
                };
                if selected.iter().any(|p| p.optional != property.optional) {
                    return None;
                }
                if property.optional && !target.optional {
                    return Some(false);
                }
                if property.optional {
                    return None;
                } // present-value inference needs optional evidence
                let value = self.indexed(check, key, depth + 1)?;
                if !self.matches_pattern(value, target.value, inferred, depth + 1)? {
                    return Some(false);
                }
            }
            return Some(true);
        }
        let pattern = self.ty(pattern, depth + 1)?;
        self.proof_value(check, depth + 1)?;
        self.proof_value(pattern, depth + 1)?;
        if let Some(pair) = arrays::relation(self.relation.lookup, self.arena(), check, pattern) {
            return match pair {
                Ok((a, b)) => self.matches_pattern(a, b, inferred, depth + 1),
                Err(()) => Some(false),
            };
        }
        if self.relation.assign(check, pattern, depth + 1) {
            return Some(true);
        }
        // A failed assignment is negative evidence only for these completely
        // known scalar domains. Nominal inequality does not imply disjointness.
        let a = self.arena().get(check);
        let b = self.arena().get(pattern);
        if matches!(
            (&a, &b),
            (
                Type::Intrinsic(Intrinsic::Undefined),
                Type::Intrinsic(Intrinsic::Void)
            )
        ) {
            return Some(true);
        }
        if matches!(
            (&a, &b),
            (
                Type::Literal(LitValue::Int(_)),
                Type::Literal(LitValue::Number(_))
            ) | (
                Type::Literal(LitValue::Number(_)),
                Type::Literal(LitValue::Int(_))
            ) | (
                Type::Literal(LitValue::Str(_)),
                Type::Literal(LitValue::Utf16(_))
            ) | (
                Type::Literal(LitValue::Utf16(_)),
                Type::Literal(LitValue::Str(_))
            )
        ) {
            return None;
        }
        if super::intersection::scalar(&a).is_some() && super::intersection::scalar(&b).is_some() {
            return Some(false);
        }
        None
    }

    pub(super) fn proof_value(&mut self, ty: TypeId, depth: usize) -> Option<()> {
        self.spend(depth)?;
        // Type::Function does not yet retain optional/rest/generic signature
        // axes. Identical parameter TypeIds are not a callable subtype proof.
        match self.arena().get(ty) {
            Type::Function { .. } | Type::Callable(_) => return None,
            Type::Apply { base, args } => {
                self.proof_value(base, depth + 1)?;
                for arg in args {
                    self.proof_value(arg, depth + 1)?;
                }
            }
            Type::Tuple(parts) | Type::Union(parts) | Type::Intersection(parts) => {
                for part in parts {
                    self.proof_value(part, depth + 1)?;
                }
            }
            Type::Object(object) => {
                for &operand in object.operands() {
                    self.proof_value(operand, depth + 1)?;
                }
            }
            Type::Operator(op) => {
                for &operand in op.operands() {
                    self.proof_value(operand, depth + 1)?;
                }
            }
            Type::Optional(inner) | Type::Constructor(inner) => {
                self.proof_value(inner, depth + 1)?
            }
            _ => {}
        }
        Some(())
    }
}

#[cfg(test)]
#[path = "program_conditional_eval_tests.rs"]
mod tests;
