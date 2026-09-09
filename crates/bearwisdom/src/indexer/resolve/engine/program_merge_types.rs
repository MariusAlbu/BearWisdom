//! Conservative ID-based relations used as merge evidence, not display equality.
use super::*;
use crate::type_checker::core::types::{Intrinsic, LitValue, TypeOperator, TypeProperty};

#[path = "program_structural_eval.rs"]
mod evaluation;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::indexer::resolve::engine::program_view) enum ArgumentRelation {
    Subtype,
    Assignable,
}

pub(in crate::indexer::resolve::engine::program_view) struct Relation<'a> {
    pub lookup: &'a Lookup<'a>,
    pub arena: &'a TypeArena,
}
impl Relation<'_> {
    pub fn valid_constructor(
        &self,
        signature: &crate::type_checker::core::types::Callable<TypeId>,
    ) -> Option<()> {
        evaluation::Eval::new(self)
            .constructor(signature, 0)
            .map(|_| ())
    }
    pub fn inference_members(&self, ty: TypeId) -> Option<Vec<super::super::nominal::Member>> {
        evaluation::Eval::new(self).comparison_members(ty, 0)
    }
    pub fn member_value(&self, member: &super::super::nominal::Member) -> Option<TypeId> {
        evaluation::Eval::new(self).member_value(member, 0)
    }
    pub fn method(
        &self,
        source: &crate::type_checker::core::types::Callable<TypeId>,
        target: &crate::type_checker::core::types::Callable<TypeId>,
    ) -> Option<bool> {
        evaluation::Eval::new(self).method(source, target)
    }
    pub fn argument(&self, from: TypeId, to: TypeId) -> Option<bool> {
        evaluation::Eval::new(self).argument(from, to)
    }
    pub fn argument_with_constraints(
        &self,
        from: TypeId,
        to: TypeId,
        constraints: impl Iterator<
            Item = (
                crate::type_checker::core::types::GenericParamId,
                Option<TypeId>,
            ),
        >,
    ) -> Option<bool> {
        self.argument_in_phase(from, to, constraints, ArgumentRelation::Assignable)
    }
    pub fn argument_in_phase(
        &self,
        from: TypeId,
        to: TypeId,
        constraints: impl Iterator<
            Item = (
                crate::type_checker::core::types::GenericParamId,
                Option<TypeId>,
            ),
        >,
        phase: ArgumentRelation,
    ) -> Option<bool> {
        let mut proof = evaluation::Eval::new(self);
        proof.subtype = phase == ArgumentRelation::Subtype;
        proof.constraints.extend(constraints);
        proof.argument(from, to)
    }
    pub(super) fn refined_property(
        &self,
        values: Vec<TypeId>,
        optional: bool,
        index: bool,
    ) -> Option<TypeId> {
        evaluation::Eval::new(self).refined_property(values, optional, index, 0)
    }
    pub fn equal(&self, left: TypeId, right: TypeId) -> bool {
        match (self.canonical(left, 0), self.canonical(right, 0)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }
    pub fn canonical(&self, ty: TypeId, depth: usize) -> Option<TypeId> {
        evaluation::Eval::new(self).ty(ty, depth)
    }
    pub fn assignable(&self, from: TypeId, to: TypeId) -> bool {
        match (self.canonical(from, 0), self.canonical(to, 0)) {
            (Some(from), Some(to)) => self.assign(from, to, 0),
            _ => false,
        }
    }
    fn assign(&self, from: TypeId, to: TypeId, depth: usize) -> bool {
        self.assign_bounded(from, to, depth, &std::cell::Cell::new(4096))
    }
    fn assign_bounded(
        &self,
        from: TypeId,
        to: TypeId,
        depth: usize,
        budget: &std::cell::Cell<usize>,
    ) -> bool {
        if depth >= 64 || budget.get() == 0 {
            return false;
        }
        budget.set(budget.get() - 1);
        let child = |from, to| self.assign_bounded(from, to, depth + 1, budget);
        if from == to {
            return true;
        }
        let a = self.arena.get(from);
        let b = self.arena.get(to);
        if matches!(a, Type::Intrinsic(Intrinsic::Never)) {
            return true;
        }
        if matches!(b, Type::Intrinsic(Intrinsic::Never)) {
            return false;
        }
        if matches!(b, Type::Intrinsic(Intrinsic::Any | Intrinsic::Unknown))
            || matches!(a, Type::Intrinsic(Intrinsic::Any))
        {
            return true;
        }
        if let Some(pair) = super::super::arrays::relation(self.lookup, self.arena, from, to) {
            return pair.is_ok_and(|(a, b)| child(a, b));
        }
        match (a, b) {
            (Type::Union(parts), _) => parts.into_iter().all(|part| child(part, to)),
            // Optional is a source union. Distribute it before choosing a target
            // union arm: its value and undefined may inhabit different arms.
            (Type::Optional(inner), _) => {
                child(inner, to)
                    && child(self.arena.intern(Type::Intrinsic(Intrinsic::Undefined)), to)
            }
            (_, Type::Union(parts)) => parts.into_iter().any(|part| child(from, part)),
            (_, Type::Optional(inner)) => {
                child(from, inner)
                    || matches!(self.arena.get(from), Type::Intrinsic(Intrinsic::Undefined))
            }
            (_, Type::Intersection(parts)) => parts.into_iter().all(|part| child(from, part)),
            (Type::Intersection(_), _) => evaluation::Eval::new(self)
                .surface(from, depth + 1)
                .is_some_and(|surface| surface != from && child(surface, to)),
            (Type::Object(a), Type::Object(b)) => {
                self.objects(&a.properties, &b.properties, depth + 1, budget)
            }
            (Type::Object(a), Type::Operator(TypeOperator::Object(b))) => {
                self.objects(&a.properties, &b, depth + 1, budget)
            }
            (Type::Operator(TypeOperator::Object(a)), Type::Object(b)) => {
                self.objects(&a, &b.properties, depth + 1, budget)
            }
            (Type::Object(_), Type::Intrinsic(Intrinsic::Object)) => true,
            (Type::Operator(TypeOperator::Object(a)), Type::Operator(TypeOperator::Object(b))) => {
                self.objects(&a, &b, depth + 1, budget)
            }
            (a, Type::Operator(TypeOperator::Object(b))) if b.is_empty() => matches!(
                a,
                Type::Literal(_)
                    | Type::UniqueSymbol(_)
                    | Type::Decl { .. }
                    | Type::Apply { .. }
                    | Type::Function { .. }
                    | Type::Tuple(_)
                    | Type::Constructor(_)
                    | Type::Intrinsic(
                        Intrinsic::Object
                            | Intrinsic::String
                            | Intrinsic::Number
                            | Intrinsic::Boolean
                            | Intrinsic::Symbol
                            | Intrinsic::BigInt
                    )
            ),
            (Type::Operator(TypeOperator::Object(_)), Type::Intrinsic(Intrinsic::Object)) => true,
            (Type::UniqueSymbol(_), Type::Intrinsic(Intrinsic::Symbol)) => true,
            (Type::Literal(literal), Type::Intrinsic(intrinsic)) => matches!(
                (literal, intrinsic),
                (LitValue::Str(_) | LitValue::Utf16(_), Intrinsic::String)
                    | (LitValue::Int(_) | LitValue::Number(_), Intrinsic::Number)
                    | (LitValue::Bool(_), Intrinsic::Boolean)
                    | (LitValue::BigInt { .. }, Intrinsic::BigInt)
            ),
            (
                Type::Decl { .. }
                | Type::Apply { .. }
                | Type::Function { .. }
                | Type::Tuple(_)
                | Type::Constructor(_),
                Type::Intrinsic(Intrinsic::Object),
            ) => true,
            _ => false,
        }
    }

    fn objects(
        &self,
        from: &[TypeProperty<TypeId>],
        to: &[TypeProperty<TypeId>],
        depth: usize,
        budget: &std::cell::Cell<usize>,
    ) -> bool {
        // A nonempty weak target still requires a common property. Empty source
        // objects are assignable to all-optional targets, not arbitrary values.
        if !from.is_empty()
            && !to.is_empty()
            && to.iter().all(|p| p.optional && !p.index)
            && !from.iter().any(|a| to.iter().any(|b| a.key == b.key))
        {
            return false;
        }
        for target in to {
            if target.index {
                for source in from {
                    // Numeric-looking string keys require ingestion evidence;
                    // missing evidence cannot silently skip a possible overlap.
                    if !source.index
                        && matches!(
                            self.arena.get(target.key),
                            Type::Intrinsic(Intrinsic::Number)
                        )
                        && matches!(
                            self.arena.get(source.key),
                            Type::Literal(LitValue::Str(_) | LitValue::Utf16(_))
                        )
                    {
                        return false;
                    }
                    if !evaluation::key_in(self.arena, source.key, target.key)
                        && !(source.index && evaluation::key_in(self.arena, target.key, source.key))
                    {
                        continue;
                    }
                    let value = if source.optional {
                        self.present_property(source)
                    } else {
                        source.value
                    };
                    if !self.assign_bounded(value, target.value, depth + 1, budget) {
                        return false;
                    }
                }
            } else if let Some(source) = from.iter().find(|p| !p.index && p.key == target.key) {
                if source.optional && !target.optional {
                    return false;
                }
                if !self.assign_bounded(
                    self.read_property(source),
                    self.read_property(target),
                    depth + 1,
                    budget,
                ) {
                    return false;
                }
            } else if !target.optional {
                return false;
            }
        }
        true
    }

    fn read_property(&self, property: &TypeProperty<TypeId>) -> TypeId {
        if property.optional {
            self.arena.intern(Type::Optional(property.value))
        } else {
            property.value
        }
    }
    fn present_property(&self, property: &TypeProperty<TypeId>) -> TypeId {
        // Implicit index compatibility examines a present optional property,
        // not the value of reading a potentially absent property.
        let undefined = self.arena.intern(Type::Intrinsic(Intrinsic::Undefined));
        let parts = match self.arena.get(property.value) {
            Type::Union(parts) => parts,
            _ => vec![property.value],
        };
        let parts: Vec<_> = parts.into_iter().filter(|&ty| ty != undefined).collect();
        match parts.len() {
            0 => self.arena.intern(Type::Intrinsic(Intrinsic::Never)),
            1 => parts[0],
            _ => self.arena.intern(Type::Union(parts)),
        }
    }
}

#[cfg(test)]
#[path = "program_merge_types_tests.rs"]
mod tests;
