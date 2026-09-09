//! Overload subtype applicability is not assignment with a different label.
use super::*;
use crate::indexer::lexical::globals::member_surface::Kind;
use crate::indexer::resolve::engine::head_decl::head_decl_id;

impl Eval<'_, '_> {
    pub(super) fn subtype_value(&mut self, from: TypeId, to: TypeId, depth: usize) -> Option<bool> {
        self.spend(depth)?;
        let from = self.ty(from, depth + 1)?;
        let to = self.ty(to, depth + 1)?;
        self.subtype_input(from, depth + 1)?;
        self.subtype_input(to, depth + 1)?;
        let a = self.arena().get(from);
        let b = self.arena().get(to);
        if from == to
            || matches!(a, Type::Intrinsic(Intrinsic::Never))
            || matches!(b, Type::Intrinsic(Intrinsic::Any | Intrinsic::Unknown))
        {
            return Some(true);
        }
        if matches!(b, Type::Intrinsic(Intrinsic::Never)) {
            return Some(false);
        }
        if let Type::Union(parts) = a {
            return self.subtype_parts(parts, to, true, depth + 1);
        }
        if let Type::Union(parts) = b {
            return self.subtype_parts(parts, from, false, depth + 1);
        }
        if matches!(a, Type::Intrinsic(Intrinsic::Any | Intrinsic::Unknown)) {
            return Some(false);
        }
        if matches!(a, Type::Intrinsic(Intrinsic::Null | Intrinsic::Undefined)) {
            let policy = self.relation.lookup.view.callable_policy?;
            if !policy.strict_nulls
                || matches!(
                    (&a, &b),
                    (
                        Type::Intrinsic(Intrinsic::Undefined),
                        Type::Intrinsic(Intrinsic::Void)
                    )
                )
            {
                return Some(true);
            }
        }
        if let Some(pair) = arrays::relation(self.relation.lookup, self.arena(), from, to) {
            return match pair {
                Ok((a, b)) => self.subtype_value(a, b, depth + 1),
                Err(()) => Some(false),
            };
        }
        match (&a, &b) {
            (Type::Callable(a), Type::Callable(b)) => {
                return self.callable_relation(a, b, depth + 1)
            }
            (Type::Callable(_), Type::Intrinsic(Intrinsic::Object)) => return Some(true),
            (Type::Callable(_), Type::Intrinsic(_) | Type::Literal(_))
            | (Type::Intrinsic(_) | Type::Literal(_), Type::Callable(_)) => return Some(false),
            (Type::Generic { param }, _) => {
                let constraint = self.constraint(*param)??;
                return match self.subtype_value(constraint, to, depth + 1) {
                    Some(true) => Some(true),
                    _ => None,
                };
            }
            (_, Type::Generic { .. }) => return None,
            (Type::Tuple(a), Type::Tuple(b)) if a.len() == b.len() => {
                let mut unknown = false;
                for (&a, &b) in a.iter().zip(b) {
                    // Optional tuple slots require presence/arity provenance.
                    if matches!(self.arena().get(a), Type::Optional(_))
                        || matches!(self.arena().get(b), Type::Optional(_))
                    {
                        return None;
                    }
                    match self.subtype_value(a, b, depth + 1) {
                        Some(false) => return Some(false),
                        None => unknown = true,
                        _ => {}
                    }
                }
                return if unknown { None } else { Some(true) };
            }
            _ => {}
        }
        if super::intersection::scalar(&a).is_some() && super::intersection::scalar(&b).is_some() {
            return self.matches_pattern(from, to, &mut FxHashMap::default(), depth + 1);
        }
        if matches!(b, Type::Intrinsic(Intrinsic::Object))
            && matches!(
                a,
                Type::Decl { .. }
                    | Type::Apply { .. }
                    | Type::Tuple(_)
                    | Type::Object(_)
                    | Type::Operator(TypeOperator::Object(_))
            )
        {
            return Some(true);
        }
        if self.nominal_pair(from, to) {
            return self.structural_members(from, to, depth + 1);
        }
        let target = self.subtype_properties(to, depth + 1)?;
        let source = self.subtype_properties(from, depth + 1)?;
        if source.iter().chain(&target).any(|p| p.index) {
            return None;
        }
        for expected in target {
            self.spend(depth)?;
            let mut found = source.iter().filter(|p| p.key == expected.key);
            // Subtyping a non-fresh object requires even optional target keys.
            // Fresh object literal arguments are not admitted by source recipes.
            let Some(actual) = found.next() else {
                return Some(false);
            };
            if found.next().is_some() {
                return None;
            }
            if actual.optional && !expected.optional {
                return Some(false);
            }
            let read = |p: &TypeProperty<TypeId>| {
                if p.optional {
                    self.arena().intern(Type::Optional(p.value))
                } else {
                    p.value
                }
            };
            if !self.subtype_value(read(actual), read(&expected), depth + 1)? {
                return Some(false);
            }
        }
        Some(true)
    }

    pub(super) fn subtype_input(&mut self, ty: TypeId, depth: usize) -> Option<()> {
        self.spend(depth)?;
        // Broad targets and identity shortcuts still need complete operands.
        // A union/object must not conceal an invalid or erased callable.
        match self.arena().get(ty) {
            Type::Callable(c) => {
                self.valid_callable(&c, self.relation.lookup.view.callable_policy?, depth + 1)?;
                for &operand in c.operands() {
                    self.subtype_input(operand, depth + 1)?;
                }
            }
            Type::Apply { base, args } => {
                self.subtype_input(base, depth + 1)?;
                for arg in args {
                    self.subtype_input(arg, depth + 1)?;
                }
            }
            Type::Tuple(parts) | Type::Union(parts) | Type::Intersection(parts) => {
                for part in parts {
                    self.subtype_input(part, depth + 1)?;
                }
            }
            Type::Object(object) => {
                for &operand in object.operands() {
                    self.subtype_input(operand, depth + 1)?;
                }
            }
            Type::Operator(op) => {
                for &operand in op.operands() {
                    self.subtype_input(operand, depth + 1)?;
                }
            }
            Type::Optional(inner) | Type::Constructor(inner) => {
                self.subtype_input(inner, depth + 1)?
            }
            Type::Intrinsic(_)
            | Type::Literal(_)
            | Type::UniqueSymbol(_)
            | Type::Generic { .. }
            | Type::Decl { .. } => {}
            _ => return None,
        }
        Some(())
    }

    fn subtype_parts(
        &mut self,
        parts: Vec<TypeId>,
        other: TypeId,
        source: bool,
        depth: usize,
    ) -> Option<bool> {
        let mut unknown = false;
        for part in parts {
            let result = if source {
                self.subtype_value(part, other, depth + 1)
            } else {
                self.subtype_value(other, part, depth + 1)
            };
            match result {
                Some(value) if value != source => return Some(value),
                None => unknown = true,
                _ => {}
            }
        }
        if unknown {
            None
        } else {
            Some(source)
        }
    }

    fn subtype_properties(
        &mut self,
        ty: TypeId,
        depth: usize,
    ) -> Option<Vec<TypeProperty<TypeId>>> {
        if head_decl_id(self.arena(), ty).is_some() {
            let members = self.nominal_members(ty, depth + 1)?;
            if members.iter().any(|m| m.kind != Kind::Property) {
                return None;
            }
            return Some(members.into_iter().map(|m| m.property).collect());
        }
        match self.arena().get(ty) {
            Type::Operator(TypeOperator::Object(properties)) => Some(properties),
            Type::Object(object) => Some(object.properties),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "program_argument_subtype_tests.rs"]
mod tests;
