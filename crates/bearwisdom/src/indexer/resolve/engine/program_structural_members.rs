//! Selected member keys connect structural inference and full argument proofs.
use super::*;
use crate::indexer::lexical::globals::member_surface::Kind;
use crate::indexer::resolve::engine::{head_decl::head_decl_id, program_view::nominal::Member};
use crate::type_checker::core::types::CallableOrigin;

impl Eval<'_, '_> {
    pub(super) fn nominal_pair(&self, from: TypeId, to: TypeId) -> bool {
        let source_owned = |ty| {
            head_decl_id(self.arena(), ty).is_some()
                || matches!(self.arena().get(ty), Type::Object(_))
        };
        source_owned(from) && source_owned(to)
    }
    pub(in super::super) fn comparison_members(
        &mut self,
        ty: TypeId,
        depth: usize,
    ) -> Option<Vec<Member>> {
        let ty = self.ty(ty, depth + 1)?;
        if let Type::Object(object) = self.arena().get(ty) {
            self.subtype_input(ty, depth + 1)?;
            return super::super::super::super::object_members::comparison(
                self.relation.lookup,
                &object,
            );
        }
        let previous = self.comparison;
        self.comparison = true;
        let result = self.nominal_members(ty, depth + 1);
        self.comparison = previous;
        result
    }

    pub(in super::super) fn member_value(
        &mut self,
        member: &Member,
        depth: usize,
    ) -> Option<TypeId> {
        let value = match member.kind {
            Kind::Property => member.property.value,
            Kind::Method => {
                let origin = CallableOrigin::new(
                    self.relation.lookup.view.context,
                    member.origin.source.ordinal(),
                    member.origin.signature.0,
                );
                self.arena().intern(Type::Callable(Box::new(
                    member.signature.callable(origin, self.arena())?,
                )))
            }
            _ => return None,
        };
        self.ty(value, depth + 1)
    }

    pub(super) fn structural_members(
        &mut self,
        from: TypeId,
        to: TypeId,
        depth: usize,
    ) -> Option<bool> {
        self.spend(depth)?;
        if !self.nominal_pair(from, to) {
            return None;
        }
        let (source, source_constructors) = self.comparison_surface(from, depth + 1)?;
        let (target, target_constructors) = self.comparison_surface(to, depth + 1)?;
        if target.iter().any(|m| m.property.index) {
            return None;
        }
        let pair = (from, to, self.subtype);
        // Only this active proof may use the recursive hypothesis. No success
        // is memoized: all leaves and sibling obligations must still succeed.
        if !self.structural_pairs.insert(pair) {
            return Some(true);
        }
        let result = (|| {
            if !self.constructor_groups(&source_constructors, &target_constructors, depth + 1)? {
                return Some(false);
            }
            self.member_groups(&source, &target, depth + 1)
        })();
        self.structural_pairs.remove(&pair);
        result
    }

    fn member_groups(
        &mut self,
        source: &[Member],
        target: &[Member],
        depth: usize,
    ) -> Option<bool> {
        if !source.is_empty()
            && !target.is_empty()
            && target.iter().all(|m| m.property.optional)
            && !source
                .iter()
                .any(|a| target.iter().any(|b| a.property.key == b.property.key))
        {
            return Some(false);
        }
        let mut unknown = false;
        for expected in target {
            self.spend(depth)?;
            let candidates: Vec<_> = source
                .iter()
                .filter(|m| !m.property.index && m.property.key == expected.property.key)
                .collect();
            if candidates.is_empty() {
                if !expected.property.optional || self.subtype {
                    return Some(false);
                }
                continue;
            }
            let mut matched = false;
            let mut incomplete = false;
            for actual in candidates {
                match self.member_pair(actual, expected, depth + 1) {
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

    fn member_pair(&mut self, source: &Member, target: &Member, depth: usize) -> Option<bool> {
        if source.property.optional && !target.property.optional {
            return Some(false);
        }
        let a = self.member_value(source, depth + 1)?;
        let b = self.member_value(target, depth + 1)?;
        if target.kind == Kind::Method {
            if let (Type::Callable(a), Type::Callable(b)) =
                (self.arena().get(a), self.arena().get(b))
            {
                return self.method_value(&a, &b, depth + 1);
            }
        }
        let read = |ty, optional| {
            if optional {
                self.arena().intern(Type::Optional(ty))
            } else {
                ty
            }
        };
        self.callable_value(
            read(a, source.property.optional),
            read(b, target.property.optional),
            self.relation.lookup.view.callable_policy?,
            depth + 1,
        )
    }
}
