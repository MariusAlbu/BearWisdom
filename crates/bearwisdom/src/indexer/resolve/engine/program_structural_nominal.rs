//! Lazy nominal queries consume source-attested member/type/generic identities.
use super::*;
use crate::indexer::lexical::globals::{member_surface::Kind, InheritedMembers};
use crate::indexer::resolve::engine::{head_decl::head_decl_id, program_view::nominal::Member};

impl Eval<'_, '_> {
    pub(super) fn nominal_members(&mut self, ty: TypeId, depth: usize) -> Option<Vec<Member>> {
        self.spend(depth)?;
        let owner = head_decl_id(self.arena(), ty)?;
        if !self.nominals.insert(owner) {
            return None;
        }
        let result = self.nominal_members_inner(ty, owner, depth + 1);
        self.nominals.remove(&owner);
        result
    }

    fn nominal_members_inner(
        &mut self,
        ty: TypeId,
        owner: i64,
        depth: usize,
    ) -> Option<Vec<Member>> {
        let input = self.relation.lookup.nominal_surface(owner)?;
        if input.incomplete
            || self.comparison
                && (input.signature_facets
                    || !self.construct_comparison && !input.constructors.is_empty())
        {
            return None;
        }
        let (bases, members, order) = (input.bases.clone(), input.members.clone(), input.order);
        let bindings = self.nominal_bindings(ty, owner, depth + 1)?;
        let mut result: Vec<_> = members
            .into_iter()
            .map(|mut member| {
                let apply = |ty| substitute(self.arena(), ty, &bindings);
                member.property.value = apply(member.property.value);
                member.signature.parameters =
                    member.signature.parameters.into_iter().map(apply).collect();
                member.signature.result = member.signature.result.map(apply);
                member.signature.constraints = member
                    .signature
                    .constraints
                    .into_iter()
                    .map(|v| v.map(apply))
                    .collect();
                member.signature.defaults = member
                    .signature
                    .defaults
                    .into_iter()
                    .map(|v| v.map(apply))
                    .collect();
                member
            })
            .collect();
        let own: FxHashSet<_> = result
            .iter()
            .map(|m| (m.property.key, m.property.index))
            .collect();
        for base in bases {
            let base = self.ty(substitute(self.arena(), base, &bindings), depth + 1)?;
            let inherited = self.nominal_members(base, depth + 1)?;
            let prior: FxHashSet<_> = result
                .iter()
                .map(|m| (m.property.key, m.property.index))
                .collect();
            for member in inherited {
                let key = (member.property.key, member.property.index);
                if own.contains(&key)
                    || order == InheritedMembers::DeclarationOrder && prior.contains(&key)
                {
                    continue;
                }
                result.push(member);
            }
        }
        Some(result)
    }

    pub(super) fn nominal_bindings(
        &mut self,
        ty: TypeId,
        owner: i64,
        depth: usize,
    ) -> Option<FxHashMap<GenericParamId, TypeId>> {
        self.spend(depth)?;
        let info = self.relation.lookup.canonical_type_info(owner)?;
        let parameters = info.generic_param_ids.clone();
        let defaults = info.generic_param_default_ids.clone();
        let mut args = match self.arena().get(ty) {
            Type::Apply { args, .. } => args,
            _ => vec![],
        };
        if args.len() > parameters.len() {
            return None;
        }
        let mut bindings: FxHashMap<_, _> = parameters
            .iter()
            .copied()
            .zip(args.iter().copied())
            .collect();
        while args.len() < parameters.len() {
            let default = defaults.get(args.len()).copied().flatten()?;
            let value = self.ty(substitute(self.arena(), default, &bindings), depth + 1)?;
            bindings.insert(parameters[args.len()], value);
            args.push(value);
        }
        for (&parameter, &argument) in parameters.iter().zip(&args) {
            if !argument_kind_agrees(self.arena(), parameter, argument) {
                return None;
            }
            if let Some(constraint) = self.constraint(parameter)? {
                let constraint =
                    self.ty(substitute(self.arena(), constraint, &bindings), depth + 1)?;
                self.obligation(argument, constraint, &mut FxHashSet::default(), depth + 1)?;
            }
        }
        Some(bindings)
    }

    pub(super) fn key_properties(
        &mut self,
        ty: TypeId,
        depth: usize,
    ) -> Option<Vec<TypeProperty<TypeId>>> {
        if head_decl_id(self.arena(), ty).is_some() {
            return Some(
                self.nominal_members(ty, depth + 1)?
                    .into_iter()
                    .map(|m| m.property)
                    .collect(),
            );
        }
        let ty = self.surface(ty, depth + 1)?;
        let Type::Operator(TypeOperator::Object(properties)) = self.arena().get(ty) else {
            return None;
        };
        Some(properties)
    }

    pub(super) fn nominal_indexed(
        &mut self,
        ty: TypeId,
        index: TypeId,
        depth: usize,
    ) -> Option<TypeId> {
        let members = self.nominal_members(ty, depth + 1)?;
        let mut selected: Vec<_> = members
            .iter()
            .filter(|m| !m.property.index && m.property.key == index)
            .collect();
        if selected.is_empty() {
            selected = members
                .iter()
                .filter(|m| m.property.index && m.property.key == index)
                .collect();
        }
        if selected.is_empty() {
            selected = members
                .iter()
                .filter(|m| m.property.index && key_in(self.arena(), index, m.property.key))
                .collect();
        }
        let first = selected.first()?;
        if selected.len() > 1 && selected.iter().any(|m| m.kind != Kind::Property) {
            return None;
        }
        let value = self.nominal_value(first, depth + 1)?;
        for member in &selected[1..] {
            // Multiple physical property declarations need an actual type
            // equality proof. Never select the first namesake as a fallback.
            if self.nominal_value(member, depth + 1)? != value {
                return None;
            }
        }
        Some(value)
    }

    fn nominal_value(&mut self, member: &Member, depth: usize) -> Option<TypeId> {
        let mut value = member.property.value;
        if member.kind == Kind::Method {
            if !member.signature.generic_parameters.is_empty()
                || member
                    .signature
                    .syntax
                    .parameters
                    .iter()
                    .any(|p| p.optional || p.rest)
            {
                return None;
            }
            value = self.arena().intern(Type::Function {
                params: member.signature.parameters.clone(),
                return_: value,
            });
        }
        let value = self.ty(value, depth + 1)?;
        if member.property.optional {
            self.union(vec![value, self.atom(Intrinsic::Undefined)], depth + 1)
        } else {
            Some(value)
        }
    }

    pub(super) fn nominal_object(&mut self, ty: TypeId, depth: usize) -> Option<TypeId> {
        let members = self.nominal_members(ty, depth + 1)?;
        let mut properties: Vec<TypeProperty<TypeId>> = Vec::new();
        for member in &members {
            self.spend(depth)?;
            let value = self.nominal_value(member, depth + 1)?;
            let property = TypeProperty {
                value,
                ..member.property.clone()
            };
            if let Some(prior) = properties
                .iter()
                .find(|p| p.key == property.key && p.index == property.index)
            {
                if member.kind != Kind::Property || prior != &property {
                    return None;
                }
            } else {
                properties.push(property);
            }
        }
        self.object(properties, depth + 1)
    }
}

#[cfg(test)]
#[path = "program_structural_nominal_tests.rs"]
mod tests;
