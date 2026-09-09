//! Non-signature value comparison used by callable parameter and result relations.
use super::*;

impl Eval<'_, '_> {
    pub(in super::super) fn callable_value(
        &mut self,
        from: TypeId,
        to: TypeId,
        policy: CallablePolicy,
        depth: usize,
    ) -> Option<bool> {
        if self.subtype {
            return self.subtype_value(from, to, depth);
        }
        self.spend(depth)?;
        let from = self.ty(from, depth + 1)?;
        let to = self.ty(to, depth + 1)?;
        match (self.arena().get(from), self.arena().get(to)) {
            (Type::Callable(_), Type::Callable(_)) => {} // comparison validates both
            (Type::Callable(c), _) | (_, Type::Callable(c)) => {
                self.valid_callable(&c, policy, depth + 1)?
            }
            _ => {}
        }
        // A generic upper bound can cover an entire union without inhabiting
        // one fixed union arm. Check that obligation before distributing.
        if matches!(self.arena().get(from), Type::Generic { .. })
            && self.proof_value(to, depth + 1).is_some()
            && self
                .obligation(from, to, &mut FxHashSet::default(), depth + 1)
                .is_some()
        {
            return Some(true);
        }
        match (self.arena().get(from), self.arena().get(to)) {
            (Type::Function { .. }, _) | (_, Type::Function { .. }) => return None,
            (Type::Callable(a), Type::Callable(b)) => {
                return self.callable_relation(&a, &b, depth + 1)
            }
            (Type::Union(parts), _) => {
                let mut unknown = false;
                for part in parts {
                    match self.callable_value(part, to, policy, depth + 1) {
                        Some(false) => return Some(false),
                        None => unknown = true,
                        _ => {}
                    }
                }
                return if unknown { None } else { Some(true) };
            }
            (_, Type::Union(parts)) => {
                let mut unknown = false;
                for part in parts {
                    match self.callable_value(from, part, policy, depth + 1) {
                        Some(true) => return Some(true),
                        None => unknown = true,
                        _ => {}
                    }
                }
                return if unknown { None } else { Some(false) };
            }
            (Type::Tuple(_), Type::Tuple(_)) => {
                return self.rest_value(from, to, policy, depth + 1)
            }
            (Type::Intrinsic(Intrinsic::Undefined | Intrinsic::Null), _)
                if !policy.strict_nulls =>
            {
                return Some(to != self.atom(Intrinsic::Never));
            }
            (
                Type::Callable(_),
                Type::Intrinsic(Intrinsic::Unknown | Intrinsic::Any | Intrinsic::Object),
            )
            | (Type::Intrinsic(Intrinsic::Never | Intrinsic::Any), Type::Callable(_)) => {
                return Some(true)
            }
            (Type::Callable(_), Type::Literal(_) | Type::Intrinsic(_))
            | (Type::Literal(_) | Type::Intrinsic(_), Type::Callable(_)) => return Some(false),
            (_, Type::Generic { param }) if self.rigid.contains(&param) && from != to => {
                if matches!(
                    self.arena().get(from),
                    Type::Literal(_)
                        | Type::Intrinsic(
                            Intrinsic::String
                                | Intrinsic::Number
                                | Intrinsic::Boolean
                                | Intrinsic::Unknown
                        )
                ) {
                    return Some(false);
                }
                if let Type::Generic { param: other } = self.arena().get(from) {
                    if self.rigid.contains(&other)
                        && self.constraint(other)?.is_none()
                        && self.constraint(param)?.is_none()
                    {
                        return Some(false);
                    }
                }
            }
            _ => {}
        }
        if let Some(pair) = arrays::relation(self.relation.lookup, self.arena(), from, to) {
            return match pair {
                Ok((a, b)) => self.callable_value(a, b, policy, depth + 1),
                Err(()) => Some(false),
            };
        }
        if self.declared_ancestor(from, to, depth + 1) == Some(true) {
            return Some(true);
        }
        if self.nominal_pair(from, to) {
            if matches!(self.arena().get(from), Type::Object(_))
                || matches!(self.arena().get(to), Type::Object(_))
            {
                return self.structural_members(from, to, depth + 1);
            }
            self.proof_value(from, depth + 1)?;
            self.proof_value(to, depth + 1)?;
            if self
                .obligation(from, to, &mut FxHashSet::default(), depth + 1)
                .is_some()
            {
                return Some(true);
            }
            return self.structural_members(from, to, depth + 1);
        }
        self.proof_value(from, depth + 1)?;
        self.proof_value(to, depth + 1)?;
        if self
            .obligation(from, to, &mut FxHashSet::default(), depth + 1)
            .is_some()
        {
            return Some(true);
        }
        self.matches_pattern(from, to, &mut FxHashMap::default(), depth + 1)
    }
}
