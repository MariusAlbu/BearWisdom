//! Mixed bases retain every operand; property refinements keep source owners.
use super::*;
use crate::type_checker::core::types::{TypeOperator, TypeProperty};

impl Proof<'_> {
    pub(super) fn base_surface(&mut self, base: TypeId) -> Option<(Vec<Fact>, Vec<TypeId>)> {
        if head_decl_id(self.relation.arena, base).is_some() {
            let (facts, application) = self.nominal_base(base)?;
            return Some((facts, vec![application]));
        }
        let mut pending = vec![base];
        let mut facts = Vec::new();
        let mut applications = Vec::new();
        let mut properties = Vec::new();
        while let Some(base) = pending.pop() {
            self.spend_base_work(1)?;
            match self.relation.arena.get(base) {
                Type::Intersection(parts) => pending.extend(parts),
                Type::Operator(TypeOperator::Object(parts)) => properties.extend(parts),
                _ if head_decl_id(self.relation.arena, base).is_some() => {
                    let (members, application) = self.nominal_base(base)?;
                    facts.extend(members);
                    applications.push(application);
                }
                _ => return None,
            }
        }
        // Object recipes currently carry type keys, not declaration origins.
        // Refinements can retain the attested nominal origin; new keys cannot.
        self.spend_base_work(facts.len().checked_add(properties.len())?)?;
        let mut groups: FxHashMap<MemberKey, Vec<Fact>> = FxHashMap::default();
        for fact in facts {
            groups.entry(fact.key).or_default().push(fact);
        }
        let mut refinements: FxHashMap<MemberKey, Vec<TypeProperty<TypeId>>> = FxHashMap::default();
        for property in properties {
            let key = if property.index {
                let Type::Intrinsic(
                    domain @ (Intrinsic::String | Intrinsic::Number | Intrinsic::Symbol),
                ) = self.relation.arena.get(property.key)
                else {
                    return None;
                };
                MemberKey::Index(domain)
            } else if let Some(&name) = self.relation.lookup.view.key_names.get(&property.key) {
                MemberKey::Named(name)
            } else if matches!(self.relation.arena.get(property.key), Type::UniqueSymbol(_)) {
                MemberKey::Unique(property.key)
            } else {
                return None;
            };
            if !groups.contains_key(&key) {
                return None;
            }
            refinements.entry(key).or_default().push(property);
        }
        let mut result = Vec::new();
        for (key, mut group) in groups {
            let refinements = refinements.remove(&key).unwrap_or_default();
            if refinements.is_empty()
                && (group.len() == 1
                    || group
                        .iter()
                        .any(|f| !matches!(f.member.kind, Kind::Property | Kind::Index)))
            {
                // Full callable evidence, including all overloads, is retained.
                result.extend(group);
                continue;
            }
            if group
                .iter()
                .any(|f| !matches!(f.member.kind, Kind::Property | Kind::Index))
            {
                return None;
            }
            let optional = group
                .iter()
                .all(|f| f.member.modifiers.contains(&Modifier::Optional))
                && refinements.iter().all(|p| p.optional);
            let readonly = group
                .iter()
                .all(|f| f.member.modifiers.contains(&Modifier::Readonly))
                && refinements.iter().all(|p| p.readonly);
            let mut values = group.iter().map(|f| f.ty).collect::<Option<Vec<_>>>()?;
            values.extend(refinements.iter().map(|p| {
                if p.optional {
                    self.relation.arena.intern(Type::Optional(p.value))
                } else {
                    p.value
                }
            }));
            let value = self.relation.refined_property(
                values,
                optional,
                matches!(key, MemberKey::Index(_)),
            )?;
            for fact in &mut group {
                fact.ty = Some(value);
                fact.member
                    .modifiers
                    .retain(|m| !matches!(m, Modifier::Optional | Modifier::Readonly));
                if optional {
                    fact.member.modifiers.push(Modifier::Optional);
                }
                if readonly {
                    fact.member.modifiers.push(Modifier::Readonly);
                }
                fact.signature.result = Some(value);
            }
            result.extend(group);
        }
        Some((result, applications))
    }

    fn spend_base_work(&mut self, amount: usize) -> Option<()> {
        let Some(remaining) = self.remaining.checked_sub(amount) else {
            self.exhausted = true;
            return None;
        };
        self.remaining = remaining;
        Some(())
    }

    fn nominal_base(&mut self, base: TypeId) -> Option<(Vec<Fact>, TypeId)> {
        let parent = head_decl_id(self.relation.arena, base)?;
        let info = self.relation.lookup.canonical_type_info(parent)?;
        let mut args = match self.relation.arena.get(base) {
            Type::Apply { args, .. } => args,
            _ => vec![],
        };
        if args.len() > info.generic_param_ids.len() {
            return None;
        }
        let parent_shape = self.shapes.get(&parent)?;
        let mut bindings: Bindings = info
            .generic_param_ids
            .iter()
            .copied()
            .zip(args.iter().copied())
            .collect();
        while args.len() < info.generic_param_ids.len() {
            let default = parent_shape
                .generics
                .as_ref()?
                .defaults
                .get(args.len())
                .copied()
                .flatten()?;
            let ty = self
                .relation
                .canonical(substitute(self.relation.arena, default, &bindings), 0)?;
            bindings.insert(info.generic_param_ids[args.len()], ty);
            args.push(ty);
        }
        if let Some(generics) = &parent_shape.generics {
            for (&argument, constraint) in args.iter().zip(&generics.constraints) {
                if let Some(constraint) = constraint {
                    let bound = substitute(self.relation.arena, *constraint, &bindings);
                    if !self.argument_satisfies(argument, bound, &mut FxHashSet::default()) {
                        return None;
                    }
                }
            }
        }
        let application = if args.is_empty() {
            base
        } else {
            let base = (self.relation.lookup as &dyn SymbolLookup)
                .declaration_type(self.relation.arena, parent)?;
            self.relation.arena.intern(Type::Apply { base, args })
        };
        let facts = self
            .effective(parent)?
            .into_iter()
            .map(|fact| self.substituted(fact, &bindings))
            .collect();
        Some((facts, application))
    }
}

#[cfg(test)]
#[path = "program_mixed_heritage_tests.rs"]
mod tests;
