//! Declared applications supply positive ancestry evidence, never name similarity.
//! Candidate views remain private until settlement removes invalid dependencies.
use super::*;
use crate::indexer::resolve::engine::head_decl::head_decl_id;

impl Eval<'_, '_> {
    pub(super) fn declared_ancestor(
        &mut self,
        from: TypeId,
        to: TypeId,
        depth: usize,
    ) -> Option<bool> {
        let (_, target, _) = self.application(to, depth + 1)?;
        self.ancestor(from, target, &mut FxHashSet::default(), depth + 1)
    }

    fn application(
        &mut self,
        ty: TypeId,
        depth: usize,
    ) -> Option<(i64, TypeId, FxHashMap<GenericParamId, TypeId>)> {
        self.spend(depth)?;
        let owner = head_decl_id(self.arena(), ty)?;
        let owner = self.relation.lookup.canonical_decl_id(owner);
        let bindings = self.nominal_bindings(ty, owner, depth + 1)?;
        let parameters = &self
            .relation
            .lookup
            .canonical_type_info(owner)?
            .generic_param_ids;
        let base =
            (self.relation.lookup as &dyn SymbolLookup).declaration_type(self.arena(), owner)?;
        let args: Vec<_> = parameters.iter().map(|p| bindings[p]).collect();
        let ty = if args.is_empty() {
            base
        } else {
            self.arena().intern(Type::Apply { base, args })
        };
        Some((owner, ty, bindings))
    }

    fn ancestor(
        &mut self,
        from: TypeId,
        target: TypeId,
        active: &mut FxHashSet<i64>,
        depth: usize,
    ) -> Option<bool> {
        self.spend(depth)?;
        let from = self.ty(from, depth + 1)?;
        if let Type::Intersection(parts) = self.arena().get(from) {
            for part in parts {
                if self.ancestor(part, target, active, depth + 1) == Some(true) {
                    return Some(true);
                }
            }
            return None;
        }
        let (owner, from, bindings) = self.application(from, depth + 1)?;
        if !active.insert(owner) {
            return None;
        }
        let result = (|| {
            let surface = self.relation.lookup.nominal_surface(owner)?;
            if surface.incomplete {
                return None;
            }
            if from == target {
                return Some(true);
            }
            let bases = surface.bases.clone();
            for base in bases {
                let base = substitute(self.arena(), base, &bindings);
                if self.ancestor(base, target, active, depth + 1) == Some(true) {
                    return Some(true);
                }
            }
            // Missing ancestry is not structural incompatibility.
            None
        })();
        active.remove(&owner);
        result
    }
}
