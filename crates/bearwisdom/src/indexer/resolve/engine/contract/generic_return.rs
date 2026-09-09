//! Generic return templates: spelling is decoded once during ingestion, never
//! during instantiation. Same-spelled parameters of different owners stay distinct.
use crate::type_checker::core::types::{
    GenericParamId, GenericParamKind, Indirection, Lifetime, Type, TypeArena, TypeId,
};
use rustc_hash::FxHashMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GenericReturn {
    parameters: Vec<GenericParamId>,
    defaults: Vec<Option<TypeId>>,
    ty: TypeId,
}

/// Ingestion boundary, also run after DB metadata has been rehydrated.
pub(crate) fn capture_all(arena: &TypeArena, slots: &mut FxHashMap<i64, super::TypeInfo>) {
    for info in slots.values_mut() {
        info.generic_return = info
            .return_type_id
            .filter(|_| !info.generic_param_ids.is_empty())
            .map(|ty| {
                GenericReturn::capture(
                    arena,
                    &info.generic_param_ids,
                    &info.generic_param_default_ids,
                    ty,
                )
            });
    }
}

impl GenericReturn {
    /// Source syntax already bound every generic occurrence to its owner ID.
    pub(crate) fn bound(
        parameters: Vec<GenericParamId>,
        defaults: Vec<Option<TypeId>>,
        ty: TypeId,
    ) -> Self {
        Self {
            parameters,
            defaults,
            ty,
        }
    }
    fn capture(
        arena: &TypeArena,
        parameters: &[GenericParamId],
        defaults: &[Option<TypeId>],
        ty: TypeId,
    ) -> Self {
        let names = parameters
            .iter()
            .map(|&param| (arena.generic_param(param).name, arena.generic_type(param)))
            .collect();
        Self {
            parameters: parameters.to_vec(),
            defaults: defaults
                .iter()
                .map(|ty| ty.map(|ty| arena.rebind_class_params(ty, &names)))
                .collect(),
            ty: arena.rebind_class_params(ty, &names),
        }
    }

    pub fn instantiate(&self, arena: &TypeArena, arguments: &[TypeId]) -> Option<TypeId> {
        if arguments.len() > self.parameters.len() {
            return None;
        }
        let mut bindings = FxHashMap::default();
        for (index, &parameter) in self.parameters.iter().enumerate() {
            let argument = arguments.get(index).copied().or_else(|| {
                self.defaults
                    .get(index)
                    .copied()
                    .flatten()
                    .map(|ty| substitute(arena, ty, &bindings))
            })?;
            if !argument_kind_agrees(arena, parameter, argument) {
                return None;
            }
            bindings.insert(parameter, argument);
        }
        Some(substitute(arena, self.ty, &bindings))
    }
}

/// Rewrite only canonical parameter identities. Decl/Class payloads are opaque:
/// their spelling cannot turn a nominal type into a generic parameter at runtime.
pub(crate) fn substitute(
    arena: &TypeArena,
    ty: TypeId,
    bindings: &FxHashMap<GenericParamId, TypeId>,
) -> TypeId {
    let rewrite = |id| substitute(arena, id, bindings);
    let rewritten = match arena.get(ty) {
        Type::Generic { param } | Type::Region(Lifetime::Parameter(param)) => {
            return bindings
                .get(&param)
                .copied()
                .map(|arg| {
                    if argument_kind_agrees(arena, param, arg) {
                        arg
                    } else {
                        arena.intern(Type::Unknown)
                    }
                })
                .unwrap_or(ty)
        }
        Type::Apply { base, args } => Type::Apply {
            base: rewrite(base),
            args: args.into_iter().map(rewrite).collect(),
        },
        Type::Function { params, return_ } => Type::Function {
            params: params.into_iter().map(rewrite).collect(),
            return_: rewrite(return_),
        },
        Type::Object(object) => {
            Type::Object(Box::new(object.map(|id| substitute(arena, *id, bindings))))
        }
        Type::Callable(c) => {
            let mut scoped = bindings.clone();
            for generic in &c.generics {
                let Type::Generic { param } = arena.get(generic.parameter) else {
                    return arena.intern(Type::Unknown);
                };
                scoped.remove(&param);
            }
            Type::Callable(Box::new(
                c.map(c.origin, |id| substitute(arena, *id, &scoped)),
            ))
        }
        Type::Tuple(items) => Type::Tuple(items.into_iter().map(rewrite).collect()),
        Type::Operator(op) => {
            if let crate::type_checker::core::types::TypeOperator::Conditional { extends, .. } = &op
            {
                let Some(parameters) = infer_parameters(arena, *extends) else {
                    return arena.intern(Type::Unknown);
                };
                if parameters.iter().any(|p| bindings.contains_key(p)) {
                    let mut scoped = bindings.clone();
                    for parameter in parameters {
                        scoped.remove(&parameter);
                    }
                    return arena
                        .intern(Type::Operator(op.map(|id| substitute(arena, *id, &scoped))));
                }
            }
            if matches!(op, crate::type_checker::core::types::TypeOperator::Infer(_)) {
                return ty;
            }
            if let crate::type_checker::core::types::TypeOperator::Mapped { parameter, .. } = &op {
                let Type::Generic { param } = arena.get(*parameter) else {
                    return arena.intern(Type::Unknown);
                };
                if bindings.contains_key(&param) {
                    let mut scoped = bindings.clone();
                    scoped.remove(&param);
                    return arena
                        .intern(Type::Operator(op.map(|id| substitute(arena, *id, &scoped))));
                }
            }
            Type::Operator(op.map(|id| rewrite(*id)))
        }
        Type::Union(items) => Type::Union(items.into_iter().map(rewrite).collect()),
        Type::Intersection(items) => Type::Intersection(items.into_iter().map(rewrite).collect()),
        Type::Optional(inner) => Type::Optional(rewrite(inner)),
        Type::Indirect {
            kind,
            mutability,
            inner,
        } => Type::Indirect {
            kind: match kind {
                Indirection::Reference(Lifetime::Parameter(param)) => {
                    Indirection::Reference(arena.region(rewrite(arena.generic_type(param))))
                }
                other => other,
            },
            mutability,
            inner: rewrite(inner),
        },
        Type::AsyncWrapper(inner) => Type::AsyncWrapper(rewrite(inner)),
        Type::Iterator(inner) => Type::Iterator(rewrite(inner)),
        Type::Constructor(inner) => Type::Constructor(rewrite(inner)),
        Type::Class(_)
        | Type::Decl { .. }
        | Type::Primitive(_)
        | Type::Intrinsic(_)
        | Type::UniqueSymbol(_)
        | Type::Literal(_)
        | Type::Region(_)
        | Type::Unknown => return ty,
    };
    arena.intern(rewritten)
}

/// Read declaration operands only, without crossing into nested conditionals.
fn infer_parameters(arena: &TypeArena, pattern: TypeId) -> Option<Vec<GenericParamId>> {
    use crate::type_checker::core::types::TypeOperator;
    let mut pending = vec![pattern];
    let mut result = Vec::new();
    let mut remaining = 4096usize;
    while let Some(ty) = pending.pop() {
        remaining = remaining.checked_sub(1)?;
        match arena.get(ty) {
            Type::Operator(TypeOperator::Conditional { .. }) => {}
            Type::Operator(TypeOperator::Infer(parameter)) => {
                let Type::Generic { param } = arena.get(parameter) else {
                    return None;
                };
                result.push(param);
            }
            Type::Operator(op) => pending.extend(op.operands().copied()),
            Type::Callable(c) => pending.extend(c.operands().copied()),
            Type::Object(object) => pending.extend(object.operands().copied()),
            Type::Apply { base, args } => {
                pending.push(base);
                pending.extend(args);
            }
            Type::Function { params, return_ } => {
                pending.extend(params);
                pending.push(return_);
            }
            Type::Tuple(parts) | Type::Union(parts) | Type::Intersection(parts) => {
                pending.extend(parts)
            }
            Type::Optional(inner) => pending.push(inner),
            _ => {}
        }
    }
    Some(result)
}

pub(crate) fn argument_kind_agrees(
    arena: &TypeArena,
    param: GenericParamId,
    argument: TypeId,
) -> bool {
    match arena.generic_kind(param) {
        GenericParamKind::Type => !matches!(arena.get(argument), Type::Region(_)),
        GenericParamKind::Lifetime => matches!(arena.get(argument), Type::Region(_)),
        GenericParamKind::Const => false,
    }
}

#[cfg(test)]
#[path = "generic_return_tests.rs"]
mod tests;
