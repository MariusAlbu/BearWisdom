//! Bounded normalization; every input was evaluated before any reduction.
use super::*;

impl Eval<'_, '_> {
    pub(in super::super) fn refined_property(
        &mut self,
        values: Vec<TypeId>,
        optional: bool,
        index: bool,
        depth: usize,
    ) -> Option<TypeId> {
        let values = values
            .into_iter()
            .map(|ty| self.ty(ty, depth + 1))
            .collect::<Option<Vec<_>>>()?;
        for &value in &values {
            self.proof_value(value, depth + 1)?;
        }
        let never = self.atom(Intrinsic::Never);
        let discriminant = !index
            && !optional
            && !values.contains(&never)
            && values
                .iter()
                .any(|&value| literal_type(self.arena(), value));
        let value = self.intersection(values, depth + 1)?;
        // A conflicting required discriminant collapses the whole base, not
        // just this member. An interface cannot extend that never surface.
        if discriminant && value == never {
            return None;
        }
        Some(value)
    }
    pub(super) fn intersection(&mut self, items: Vec<TypeId>, depth: usize) -> Option<TypeId> {
        self.spend(depth)?;
        let mut pending = items;
        let mut parts = Vec::new();
        while let Some(part) = pending.pop() {
            self.spend(depth)?;
            match self.arena().get(part) {
                Type::Intersection(items) => pending.extend(items),
                _ => parts.push(part),
            }
        }
        parts.sort_unstable();
        parts.dedup();
        let mut result = self.atom(Intrinsic::Unknown);
        for part in parts {
            result = self.meet(result, part, depth + 1)?;
        }
        Some(result)
    }

    fn meet(&mut self, a: TypeId, b: TypeId, depth: usize) -> Option<TypeId> {
        self.spend(depth)?;
        if a == b {
            return Some(a);
        }
        let never = self.atom(Intrinsic::Never);
        let unknown = self.atom(Intrinsic::Unknown);
        if a == never || b == never {
            return Some(never);
        }
        if a == unknown {
            return Some(b);
        }
        if b == unknown {
            return Some(a);
        }
        if let Type::Union(parts) = self.arena().get(a) {
            let parts = parts
                .into_iter()
                .map(|part| self.meet(part, b, depth + 1))
                .collect::<Option<Vec<_>>>()?;
            return self.union(parts, depth + 1);
        }
        if let Type::Union(parts) = self.arena().get(b) {
            let parts = parts
                .into_iter()
                .map(|part| self.meet(a, part, depth + 1))
                .collect::<Option<Vec<_>>>()?;
            return self.union(parts, depth + 1);
        }
        let left = self.arena().get(a);
        let right = self.arena().get(b);
        let empty =
            |ty: &Type| matches!(ty, Type::Operator(TypeOperator::Object(p)) if p.is_empty());
        if empty(&left) || empty(&right) {
            let other = if empty(&left) { b } else { a };
            if matches!(
                self.arena().get(other),
                Type::Intrinsic(Intrinsic::Null | Intrinsic::Undefined | Intrinsic::Void)
            ) {
                return Some(never);
            }
            if self
                .obligation(
                    other,
                    if empty(&left) { a } else { b },
                    &mut FxHashSet::default(),
                    depth + 1,
                )
                .is_some()
            {
                return Some(other);
            }
        }
        if let (Some(ak), Some(bk)) = (scalar(&left), scalar(&right)) {
            if matches!(
                (ak, bk),
                (Intrinsic::Void, Intrinsic::Undefined) | (Intrinsic::Undefined, Intrinsic::Void)
            ) {
                return Some(self.atom(Intrinsic::Undefined));
            }
            if ak != bk {
                return Some(never);
            }
            return Some(match (&left, &right) {
                (
                    Type::Literal(_) | Type::UniqueSymbol(_),
                    Type::Literal(_) | Type::UniqueSymbol(_),
                ) => never,
                (Type::Literal(_) | Type::UniqueSymbol(_), _) => a,
                (_, Type::Literal(_) | Type::UniqueSymbol(_)) => b,
                _ => return None,
            });
        }
        // A mixed nominal/structural intersection is not a proved surface.
        // Keep every branch; inheritance must explicitly project and prove it.
        let mut parts = Vec::new();
        for ty in [a, b] {
            match self.arena().get(ty) {
                Type::Intersection(items) => parts.extend(items),
                _ => parts.push(ty),
            }
        }
        parts.sort_unstable();
        parts.dedup();
        let ty = self.arena().intern(Type::Intersection(parts.clone()));
        if parts.iter().all(|&part| {
            matches!(
                self.arena().get(part),
                Type::Operator(TypeOperator::Object(_))
            )
        }) {
            // Assignment surfaces are not declaration identity. Only an actual
            // discriminant reduction replaces this intersection with never.
            if self.surface(ty, depth + 1)? == never {
                return Some(never);
            }
        }
        Some(ty)
    }

    pub(in super::super) fn surface(&mut self, ty: TypeId, depth: usize) -> Option<TypeId> {
        self.spend(depth)?;
        if crate::indexer::resolve::engine::head_decl::head_decl_id(self.arena(), ty).is_some() {
            return self.nominal_object(ty, depth + 1);
        }
        let parts = match self.arena().get(ty) {
            Type::Object(object) => {
                return Some(
                    self.arena()
                        .intern(Type::Operator(TypeOperator::Object(object.properties))),
                )
            }
            Type::Operator(TypeOperator::Object(_)) => return Some(ty),
            Type::Intersection(parts) => parts,
            _ => return None,
        };
        let mut groups: FxHashMap<(TypeId, bool), Vec<TypeProperty<TypeId>>> = FxHashMap::default();
        for part in parts {
            self.spend(depth)?;
            let Type::Operator(TypeOperator::Object(properties)) = self.arena().get(part) else {
                return None;
            };
            for property in properties {
                groups
                    .entry((property.key, property.index))
                    .or_default()
                    .push(property);
            }
        }
        let never = self.atom(Intrinsic::Never);
        let mut properties = Vec::new();
        for ((key, index), group) in groups {
            let optional = group.iter().all(|p| p.optional);
            let readonly = group.iter().all(|p| p.readonly);
            let values = group
                .iter()
                .map(|p| self.ty(self.relation.read_property(p), depth + 1))
                .collect::<Option<Vec<_>>>()?;
            let has_literal = values
                .iter()
                .any(|&value| literal_type(self.arena(), value));
            let has_never = values.contains(&never);
            let value = self.intersection(values, depth + 1)?;
            if !index && !optional && has_literal && !has_never && value == never {
                return Some(never);
            }
            properties.push(TypeProperty {
                key,
                value,
                optional,
                readonly,
                index,
            });
        }
        self.object(properties, depth + 1)
    }
}

fn literal_type(arena: &TypeArena, ty: TypeId) -> bool {
    let unit = |ty| {
        matches!(
            arena.get(ty),
            Type::Literal(_)
                | Type::UniqueSymbol(_)
                | Type::Intrinsic(Intrinsic::Null | Intrinsic::Undefined)
        )
    };
    match arena.get(ty) {
        Type::Union(parts) => parts.into_iter().all(unit),
        Type::Intrinsic(Intrinsic::Boolean) => true,
        _ => unit(ty),
    }
}

pub(super) fn scalar(ty: &Type) -> Option<Intrinsic> {
    match ty {
        Type::Intrinsic(
            kind @ (Intrinsic::String
            | Intrinsic::Number
            | Intrinsic::Boolean
            | Intrinsic::Symbol
            | Intrinsic::BigInt
            | Intrinsic::Null
            | Intrinsic::Undefined
            | Intrinsic::Void),
        ) => Some(*kind),
        Type::Literal(LitValue::Str(_) | LitValue::Utf16(_)) => Some(Intrinsic::String),
        Type::Literal(LitValue::Int(_) | LitValue::Number(_)) => Some(Intrinsic::Number),
        Type::Literal(LitValue::Bool(_)) => Some(Intrinsic::Boolean),
        Type::Literal(LitValue::BigInt { .. }) => Some(Intrinsic::BigInt),
        Type::UniqueSymbol(_) => Some(Intrinsic::Symbol),
        _ => None,
    }
}

#[cfg(test)]
#[path = "program_structural_intersection_tests.rs"]
mod tests;
