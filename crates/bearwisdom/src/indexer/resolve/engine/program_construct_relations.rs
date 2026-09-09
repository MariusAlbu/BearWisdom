//! Constructor facets retain source origins independently of named property keys.
use super::*;
use crate::indexer::resolve::engine::{head_decl::head_decl_id, program_view::nominal::Member};
use crate::type_checker::core::types::{Callable, CallableOrigin};

impl Eval<'_, '_> {
    pub(in super::super) fn constructor(
        &mut self,
        signature: &Callable<TypeId>,
        depth: usize,
    ) -> Option<Callable<TypeId>> {
        self.spend(depth)?;
        let ty = self
            .arena()
            .intern(Type::Callable(Box::new(signature.clone())));
        if !(self.relation.lookup as &dyn SymbolLookup).accepts_type_context(self.arena(), ty) {
            return None;
        }
        if signature.predicate.is_some() || signature.parameters.iter().any(|p| p.receiver) {
            return None;
        }
        let mut optional = false;
        let mut slots = FxHashSet::default();
        for (index, p) in signature.parameters.iter().enumerate() {
            if !slots.insert(p.declaration)
                || p.rest && (p.optional || index + 1 != signature.parameters.len())
            {
                return None;
            }
            if optional && !p.optional && !p.rest {
                return None;
            }
            optional |= p.optional;
        }
        let signature = self.canonical_callable(signature, depth + 1)?;
        // Inheriting an inventory does not compare signature variance. Only a
        // constrained default needs an argument proof here; comparisons below
        // still require the selected callable policy in callable_relation.
        if signature
            .generics
            .iter()
            .any(|g| g.default.is_some() && g.constraint.is_some())
        {
            self.valid_callable(
                &signature,
                self.relation.lookup.view.callable_policy?,
                depth + 1,
            )?;
        }
        Some(signature)
    }

    pub(super) fn comparison_surface(
        &mut self,
        ty: TypeId,
        depth: usize,
    ) -> Option<(Vec<Member>, Vec<Callable<TypeId>>)> {
        let previous = self.construct_comparison;
        self.construct_comparison = true;
        let members = self.comparison_members(ty, depth + 1);
        self.construct_comparison = previous;
        Some((members?, self.constructors(ty, depth + 1)?))
    }

    fn constructors(&mut self, ty: TypeId, depth: usize) -> Option<Vec<Callable<TypeId>>> {
        self.spend(depth)?;
        let ty = self.ty(ty, depth + 1)?;
        if matches!(self.arena().get(ty), Type::Object(_)) {
            return Some(vec![]);
        }
        let owner = head_decl_id(self.arena(), ty)?;
        if !self.nominals.insert(owner) {
            return None;
        }
        let result = self.constructors_inner(ty, owner, depth + 1);
        self.nominals.remove(&owner);
        result
    }

    fn constructors_inner(
        &mut self,
        ty: TypeId,
        owner: i64,
        depth: usize,
    ) -> Option<Vec<Callable<TypeId>>> {
        let input = self.relation.lookup.nominal_surface(owner)?;
        if input.incomplete || input.signature_facets {
            return None;
        }
        let (constructors, bases) = (input.constructors.clone(), input.bases.clone());
        let bindings = self.nominal_bindings(ty, owner, depth + 1)?;
        let mut result = Vec::new();
        for constructor in constructors {
            self.spend(depth)?;
            let origin = CallableOrigin::new(
                self.relation.lookup.view.context,
                constructor.origin.source.ordinal(),
                constructor.origin.signature.0,
            );
            let callable = constructor.signature.callable(origin, self.arena())?;
            let callable = callable.map(origin, |&ty| substitute(self.arena(), ty, &bindings));
            result.push(self.constructor(&callable, depth + 1)?);
        }
        for base in bases {
            let base = substitute(self.arena(), base, &bindings);
            result.extend(self.constructors(base, depth + 1)?);
        }
        Some(result)
    }

    pub(super) fn constructor_groups(
        &mut self,
        source: &[Callable<TypeId>],
        target: &[Callable<TypeId>],
        depth: usize,
    ) -> Option<bool> {
        let mut unknown = false;
        for expected in target {
            self.spend(depth)?;
            let mut matched = false;
            let mut incomplete = false;
            for actual in source {
                // ConstructSignature uses ordinary variance; MethodSignature's
                // bivariance exemption cannot leak into this proof.
                match self.callable_relation(actual, expected, depth + 1) {
                    Some(true) => {
                        matched = true;
                        break;
                    }
                    None => incomplete = true,
                    _ => {}
                }
            }
            if !matched && !incomplete {
                return Some(false);
            }
            unknown |= !matched;
        }
        if unknown {
            None
        } else {
            Some(true)
        }
    }
}

#[cfg(test)]
#[path = "program_construct_relations_tests.rs"]
mod tests;
