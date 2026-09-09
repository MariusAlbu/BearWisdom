//! Bounded evaluation of source-bound structural recipes for private proofs.
use super::super::super::arrays;
use super::*;
use crate::indexer::resolve::engine::contract::generic_return::{argument_kind_agrees, substitute};
use crate::type_checker::core::types::{GenericParamId, MappedModifier};
use rustc_hash::{FxHashMap, FxHashSet};

#[path = "program_declared_ancestors.rs"]
mod ancestors;
#[path = "program_conditional_eval.rs"]
mod conditional;
#[path = "program_construct_relations.rs"]
mod constructs;
#[path = "program_structural_intersection.rs"]
mod intersection;
#[path = "program_structural_members.rs"]
mod members;
#[path = "program_structural_nominal.rs"]
mod nominal;
#[path = "program_structural_ops.rs"]
mod operators;

#[path = "program_callable_relations.rs"]
mod callables;

#[path = "program_argument_subtype.rs"]
mod argument_subtype;

pub(super) struct Eval<'r, 'a> {
    pub(super) subtype: bool,
    relation: &'r Relation<'a>,
    active: FxHashSet<TypeId>,
    aliases: FxHashSet<i64>,
    nominals: FxHashSet<i64>,
    comparison: bool,
    construct_comparison: bool,
    structural_pairs: FxHashSet<(TypeId, TypeId, bool)>,
    rigid: FxHashSet<GenericParamId>,
    pub(super) constraints: FxHashMap<GenericParamId, Option<TypeId>>,
    remaining: usize,
}

impl<'r, 'a> Eval<'r, 'a> {
    pub(super) fn new(relation: &'r Relation<'a>) -> Self {
        Self {
            subtype: false,
            relation,
            active: Default::default(),
            aliases: Default::default(),
            nominals: Default::default(),
            comparison: false,
            construct_comparison: false,
            structural_pairs: Default::default(),
            rigid: Default::default(),
            constraints: Default::default(),
            remaining: 4096,
        }
    }
    fn spend(&mut self, depth: usize) -> Option<()> {
        if depth >= 64 {
            return None;
        }
        self.remaining = self.remaining.checked_sub(1)?;
        Some(())
    }
    fn arena(&self) -> &TypeArena {
        self.relation.arena
    }
    fn constraint(&self, parameter: GenericParamId) -> Option<Option<TypeId>> {
        self.constraints
            .get(&parameter)
            .copied()
            .or_else(|| self.relation.lookup.generic_constraint(parameter))
    }
    pub(super) fn ty(&mut self, ty: TypeId, depth: usize) -> Option<TypeId> {
        self.spend(depth)?;
        if !(self.relation.lookup as &dyn SymbolLookup).accepts_type_context(self.arena(), ty)
            || !self.active.insert(ty)
        {
            return None;
        }
        let result = self.evaluate(ty, depth);
        self.active.remove(&ty);
        result
    }
    fn evaluate(&mut self, ty: TypeId, depth: usize) -> Option<TypeId> {
        let head = match self.arena().get(ty) {
            Type::Apply { base, .. } => base,
            _ => ty,
        };
        if let Type::Decl { symbol_id, .. } = self.arena().get(head) {
            let id = self.relation.lookup.canonical_decl_id(symbol_id);
            self.relation.lookup.symbol_by_id(id)?;
            if self
                .relation
                .lookup
                .canonical_type_info(id)
                .is_some_and(|info| info.lexical_alias.is_some())
            {
                return self.alias(ty, id, depth + 1);
            }
            if self.relation.lookup.symbol_by_id(id)?.kind == "type_alias" {
                return None;
            }
        }
        let value = match self.arena().get(ty) {
            Type::Generic { param } => {
                self.relation.lookup.generic_constraint(param)?;
                return Some(ty);
            }
            Type::Decl { .. } | Type::Intrinsic(_) | Type::Literal(_) | Type::UniqueSymbol(_) => {
                return Some(ty)
            }
            Type::Apply { base, args } => Type::Apply {
                base: self.ty(base, depth + 1)?,
                args: self.items(args, depth + 1)?,
            },
            Type::Function { params, return_ } => Type::Function {
                params: self.items(params, depth + 1)?,
                return_: self.ty(return_, depth + 1)?,
            },
            Type::Callable(c) => Type::Callable(Box::new(self.canonical_callable(&c, depth + 1)?)),
            Type::Object(mut object) => {
                self.relation.lookup.view.objects.members(object.origin)?;
                for property in &mut object.properties {
                    property.key = self.ty(property.key, depth + 1)?;
                    property.value = self.ty(property.value, depth + 1)?;
                }
                Type::Object(object)
            }
            Type::Tuple(items) => Type::Tuple(self.items(items, depth + 1)?),
            Type::Constructor(inner) => Type::Constructor(self.ty(inner, depth + 1)?),
            Type::Optional(inner) => {
                let inner = self.ty(inner, depth + 1)?;
                return self.union(vec![inner, self.atom(Intrinsic::Undefined)], depth + 1);
            }
            Type::Union(items) => {
                let items = self.items(items, depth + 1)?;
                return self.union(items, depth + 1);
            }
            Type::Intersection(items) => {
                let items = self.items(items, depth + 1)?;
                return self.intersection(items, depth + 1);
            }
            Type::Operator(op) => return self.operator(op, depth + 1),
            _ => return None,
        };
        Some(self.arena().intern(value))
    }
    fn items(&mut self, items: Vec<TypeId>, depth: usize) -> Option<Vec<TypeId>> {
        items.into_iter().map(|ty| self.ty(ty, depth)).collect()
    }
    fn alias(&mut self, ty: TypeId, id: i64, depth: usize) -> Option<TypeId> {
        let info = self.relation.lookup.canonical_type_info(id)?;
        let template = info.lexical_alias.clone()?;
        let parameters = info.generic_param_ids.clone();
        let defaults = info.generic_param_default_ids.clone();
        let arguments = match self.arena().get(ty) {
            Type::Apply { args, .. } => args,
            _ => vec![],
        };
        if arguments.len() > parameters.len() {
            return None;
        }
        // Nested applications in supplied arguments are not recursive aliases.
        let mut arguments = self.items(arguments, depth + 1)?;
        if !self.aliases.insert(id) {
            return None;
        }
        let result = (|| {
            let mut bindings: FxHashMap<_, _> = parameters
                .iter()
                .copied()
                .zip(arguments.iter().copied())
                .collect();
            while arguments.len() < parameters.len() {
                let default = defaults.get(arguments.len()).copied().flatten()?;
                let argument = self.ty(substitute(self.arena(), default, &bindings), depth + 1)?;
                bindings.insert(parameters[arguments.len()], argument);
                arguments.push(argument);
            }
            for (&parameter, &argument) in parameters.iter().zip(&arguments) {
                if !argument_kind_agrees(self.arena(), parameter, argument) {
                    return None;
                }
                if let Some(constraint) = self.constraint(parameter)? {
                    let constraint =
                        self.ty(substitute(self.arena(), constraint, &bindings), depth + 1)?;
                    self.obligation(argument, constraint, &mut FxHashSet::default(), depth + 1)?;
                }
            }
            self.ty(template.instantiate(self.arena(), &arguments)?, depth + 1)
        })();
        self.aliases.remove(&id);
        result
    }
    fn obligation(
        &mut self,
        argument: TypeId,
        constraint: TypeId,
        seen: &mut FxHashSet<GenericParamId>,
        depth: usize,
    ) -> Option<()> {
        self.spend(depth)?;
        if self.relation.assign(argument, constraint, depth) {
            return Some(());
        }
        let Type::Generic { param } = self.arena().get(argument) else {
            return None;
        };
        if !seen.insert(param) {
            return None;
        }
        let upper = self.constraint(param)??;
        let upper = self.ty(upper, depth + 1)?;
        self.obligation(upper, constraint, seen, depth + 1)
    }
    fn atom(&self, atom: Intrinsic) -> TypeId {
        self.arena().intern(Type::Intrinsic(atom))
    }
    fn union(&mut self, items: Vec<TypeId>, depth: usize) -> Option<TypeId> {
        self.spend(depth)?;
        let mut pending = items;
        let mut result = Vec::new();
        while let Some(item) = pending.pop() {
            self.spend(depth)?;
            match self.arena().get(item) {
                Type::Union(items) => pending.extend(items),
                Type::Intrinsic(Intrinsic::Never) => {}
                _ => result.push(item),
            }
        }
        result.sort_unstable();
        result.dedup();
        Some(match result.len() {
            0 => self.atom(Intrinsic::Never),
            1 => result[0],
            _ => self.arena().intern(Type::Union(result)),
        })
    }
}

pub(super) fn key_in(arena: &TypeArena, key: TypeId, domain: TypeId) -> bool {
    key == domain
        || matches!(
            (arena.get(key), arena.get(domain)),
            (
                Type::Literal(LitValue::Str(_) | LitValue::Utf16(_)),
                Type::Intrinsic(Intrinsic::String)
            ) | (
                Type::Intrinsic(Intrinsic::Number),
                Type::Intrinsic(Intrinsic::String)
            ) | (Type::UniqueSymbol(_), Type::Intrinsic(Intrinsic::Symbol))
        )
}

#[cfg(test)]
#[path = "program_structural_eval_tests.rs"]
mod tests;
