//! Retained import-dependent signatures. Rebinding traverses IDs, never type text.
use super::{contract::TypeInfo, lexical_type_ids::TypeBinder};
use crate::indexer::{
    lexical::{type_syntax::TypeExpr, BindingId},
    symbol_ids::SymbolIds,
};
use crate::type_checker::core::types::{Indirection, Type, TypeArena, TypeId};
use crate::types::ParsedFile;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum Recipe {
    Fixed(TypeId),
    Import(usize),
    Apply(Box<Self>, Vec<Self>),
    Function(Vec<Self>, Box<Self>),
    Operator(Box<crate::type_checker::core::types::TypeOperator<Self>>),
    Source {
        binding: usize,
        local: bool,
        legacy: Option<TypeId>,
    },
    SourceParameter {
        binding: usize,
        index: usize,
    },
    InputRegion {
        owner: i64,
        byte: u32,
    },
    InputApplication {
        owner: i64,
        byte: u32,
        base: Box<Self>,
        args: Vec<Self>,
    },
    Output {
        inputs: Vec<Self>,
        result: Box<Self>,
    },
    OutputRegion,
    OutputApplication {
        base: Box<Self>,
        args: Vec<Self>,
    },
    Tuple(Vec<Self>),
    Union(Vec<Self>),
    Intersection(Vec<Self>),
    Optional(Box<Self>),
    Indirect {
        kind: Indirection,
        mutability: crate::type_checker::core::types::Mutability,
        #[serde(default)]
        region: Option<Box<Self>>,
        inner: Box<Self>,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) enum Slot {
    Field,
    Return,
    Alias,
    Parameters,
    Receiver,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Signature {
    pub declaration: i64,
    pub slot: Slot,
    pub recipes: Vec<Recipe>,
}

pub(super) fn capture(
    file: &ParsedFile,
    ids: &SymbolIds,
    lookup: &super::compilation::Compilation,
    arena: &TypeArena,
) -> Vec<Signature> {
    let Some(graph) = file.flow.lexical.as_ref() else {
        return Vec::new();
    };
    let binder = TypeBinder {
        graph,
        path: &file.path,
        ids,
        lookup,
        source: Some(lookup),
        arena,
    };
    let mut out = Vec::new();
    for (slot, recipes) in [
        (Slot::Field, &graph.types.fields),
        (Slot::Return, &graph.types.returns),
        (Slot::Alias, &graph.types.aliases),
        (Slot::Receiver, &graph.types.receivers),
    ] {
        for (&row, recipe) in recipes {
            if let Some(declaration) = ids
                .row_id(&file.path, row)
                .filter(|_| depends_on_import(recipe, &binder))
            {
                out.push(Signature {
                    declaration,
                    slot,
                    recipes: vec![lower(recipe, &binder)],
                });
            }
        }
    }
    for (&row, recipes) in &graph.types.parameters {
        if let Some(declaration) = ids
            .row_id(&file.path, row)
            .filter(|_| recipes.iter().any(|r| depends_on_import(r, &binder)))
        {
            out.push(Signature {
                declaration,
                slot: Slot::Parameters,
                recipes: recipes.iter().map(|r| lower(r, &binder)).collect(),
            });
        }
    }
    out
}

fn depends_on_import(recipe: &TypeExpr, binder: &TypeBinder) -> bool {
    match recipe {
        TypeExpr::Declaration(binding) => binder.graph.module.imports.contains_key(binding),
        TypeExpr::Source { .. }
        | TypeExpr::SourceParameter { .. }
        | TypeExpr::InputRegion { .. }
        | TypeExpr::InputApplication { .. } => true,
        TypeExpr::Output { .. } | TypeExpr::OutputRegion | TypeExpr::OutputApplication { .. } => {
            true
        }
        TypeExpr::Apply(base, args) => {
            depends_on_import(base, binder) || args.iter().any(|r| depends_on_import(r, binder))
        }
        TypeExpr::Function(args, ret) => {
            depends_on_import(ret, binder) || args.iter().any(|r| depends_on_import(r, binder))
        }
        TypeExpr::Callable(_, legacy) => depends_on_import(legacy, binder),
        TypeExpr::Optional(inner) => depends_on_import(inner, binder),
        TypeExpr::Operator(op) => op.operands().any(|r| depends_on_import(r, binder)),
        TypeExpr::Indirect { inner, region, .. } => {
            depends_on_import(inner, binder)
                || region
                    .as_ref()
                    .is_some_and(|r| depends_on_import(r, binder))
        }
        TypeExpr::Tuple(items) | TypeExpr::Union(items) | TypeExpr::Intersection(items) => {
            items.iter().any(|r| depends_on_import(r, binder))
        }
        _ => false,
    }
}

pub(super) fn lower(recipe: &TypeExpr, binder: &TypeBinder) -> Recipe {
    if !depends_on_import(recipe, binder) {
        return Recipe::Fixed(binder.materialize(recipe));
    }
    let child = |r| lower(r, binder);
    match recipe {
        TypeExpr::Declaration(binding) => Recipe::Import(binding.0),
        TypeExpr::SourceParameter { owner, index } => Recipe::SourceParameter {
            binding: owner.binding.0,
            index: *index,
        },
        TypeExpr::Source { usage, legacy } => Recipe::Source {
            binding: usage.binding.0,
            local: usage.local,
            legacy: legacy
                .as_ref()
                .map(|legacy| binder.arena.intern_type_str(legacy)),
        },
        TypeExpr::Apply(base, args) => {
            Recipe::Apply(Box::new(child(base)), args.iter().map(child).collect())
        }
        TypeExpr::Operator(op) => Recipe::Operator(Box::new(op.map(child))),
        TypeExpr::Output { inputs, result } => Recipe::Output {
            inputs: inputs.iter().map(child).collect(),
            result: Box::new(child(result)),
        },
        TypeExpr::OutputRegion => Recipe::OutputRegion,
        TypeExpr::OutputApplication { base, args } => Recipe::OutputApplication {
            base: Box::new(child(base)),
            args: args.iter().map(child).collect(),
        },
        TypeExpr::InputRegion { owner, byte } => binder
            .ids
            .row_id(binder.path, *owner)
            .map(|owner| Recipe::InputRegion { owner, byte: *byte })
            .unwrap_or_else(|| Recipe::Fixed(binder.arena.intern(Type::Unknown))),
        TypeExpr::InputApplication {
            owner,
            byte,
            base,
            args,
        } => binder
            .ids
            .row_id(binder.path, *owner)
            .map(|owner| Recipe::InputApplication {
                owner,
                byte: *byte,
                base: Box::new(child(base)),
                args: args.iter().map(child).collect(),
            })
            .unwrap_or_else(|| Recipe::Fixed(binder.arena.intern(Type::Unknown))),
        TypeExpr::Function(args, ret) => {
            Recipe::Function(args.iter().map(child).collect(), Box::new(child(ret)))
        }
        TypeExpr::Callable(_, legacy) => child(legacy),
        TypeExpr::Tuple(items) => Recipe::Tuple(items.iter().map(child).collect()),
        TypeExpr::Union(items) => Recipe::Union(items.iter().map(child).collect()),
        TypeExpr::Intersection(items) => Recipe::Intersection(items.iter().map(child).collect()),
        TypeExpr::Optional(inner) => Recipe::Optional(Box::new(child(inner))),
        TypeExpr::Indirect {
            kind,
            mutability,
            region,
            inner,
        } => Recipe::Indirect {
            kind: *kind,
            mutability: *mutability,
            region: region.as_ref().map(|r| Box::new(child(r))),
            inner: Box::new(child(inner)),
        },
        _ => Recipe::Fixed(binder.materialize(recipe)),
    }
}

impl Recipe {
    pub fn materialize(
        &self,
        arena: &TypeArena,
        import: &impl Fn(BindingId) -> Option<TypeId>,
    ) -> TypeId {
        self.materialize_with_parameters(arena, import, &|_, _| None)
    }
    pub fn materialize_with_parameters(
        &self,
        arena: &TypeArena,
        import: &impl Fn(BindingId) -> Option<TypeId>,
        parameter: &impl Fn(BindingId, usize) -> Option<TypeId>,
    ) -> TypeId {
        self.materialize_with_context(arena, import, parameter, None)
    }
    pub fn materialize_with_context(
        &self,
        arena: &TypeArena,
        import: &impl Fn(BindingId) -> Option<TypeId>,
        parameter: &impl Fn(BindingId, usize) -> Option<TypeId>,
        lookup: Option<&dyn super::contract::SymbolLookup>,
    ) -> TypeId {
        self.materialize_output(
            arena,
            import,
            parameter,
            lookup,
            crate::type_checker::core::types::Lifetime::Unknown,
        )
    }
    fn materialize_output(
        &self,
        arena: &TypeArena,
        import: &impl Fn(BindingId) -> Option<TypeId>,
        parameter: &impl Fn(BindingId, usize) -> Option<TypeId>,
        lookup: Option<&dyn super::contract::SymbolLookup>,
        output: crate::type_checker::core::types::Lifetime,
    ) -> TypeId {
        let child = |r: &Self| r.materialize_output(arena, import, parameter, lookup, output);
        arena.intern(match self {
            Self::Fixed(ty) => return *ty,
            Self::SourceParameter { binding, index } => {
                return parameter(BindingId(*binding), *index)
                    .unwrap_or_else(|| arena.intern(Type::Unknown))
            }
            Self::Import(binding) => {
                return import(BindingId(*binding)).unwrap_or_else(|| arena.intern(Type::Unknown))
            }
            // None means only an unconfigured external boundary. An attested
            // missing/ambiguous target is Some(Unknown), never legacy fallback.
            Self::Source {
                binding,
                local,
                legacy,
            } => {
                return import(BindingId(*binding))
                    .or_else(|| legacy.filter(|_| !*local))
                    .unwrap_or_else(|| arena.intern(Type::Unknown))
            }
            Self::Apply(base, args) => Type::Apply {
                base: child(base),
                args: args.iter().map(child).collect(),
            },
            Self::Operator(op) => Type::Operator(op.map(child)),
            Self::Output { inputs, result } => {
                let inputs: Vec<_> = inputs
                    .iter()
                    .map(|r| r.materialize_with_context(arena, import, parameter, lookup))
                    .collect();
                let region = lookup
                    .map(|lookup| super::output_lifetimes::unique(lookup, arena, &inputs))
                    .unwrap_or(crate::type_checker::core::types::Lifetime::Unknown);
                return result.materialize_output(arena, import, parameter, lookup, region);
            }
            Self::OutputRegion => Type::Region(output),
            Self::OutputApplication { base, args } => {
                return lookup
                    .map(|lookup| {
                        super::output_lifetimes::application(
                            lookup,
                            arena,
                            output,
                            child(base),
                            args.iter().map(child).collect(),
                        )
                    })
                    .unwrap_or_else(|| arena.intern(Type::Unknown))
            }
            Self::InputRegion { owner, byte } => {
                return lookup
                    .map(|lookup| super::elided_inputs::region(lookup, arena, *owner, *byte, 0))
                    .unwrap_or_else(|| {
                        arena.intern(Type::Region(
                            crate::type_checker::core::types::Lifetime::Unknown,
                        ))
                    })
            }
            Self::InputApplication {
                owner,
                byte,
                base,
                args,
            } => {
                return lookup
                    .map(|lookup| {
                        super::elided_inputs::application(
                            lookup,
                            arena,
                            *owner,
                            *byte,
                            child(base),
                            args.iter().map(child).collect(),
                        )
                    })
                    .unwrap_or_else(|| arena.intern(Type::Unknown))
            }
            Self::Function(args, ret) => Type::Function {
                params: args.iter().map(child).collect(),
                return_: child(ret),
            },
            Self::Tuple(items) => Type::Tuple(items.iter().map(child).collect()),
            Self::Union(items) => Type::Union(items.iter().map(child).collect()),
            Self::Intersection(items) => Type::Intersection(items.iter().map(child).collect()),
            Self::Optional(inner) => Type::Optional(child(inner)),
            Self::Indirect {
                kind,
                mutability,
                region,
                inner,
            } => Type::Indirect {
                kind: match (kind, region) {
                    (Indirection::Reference(_), Some(region)) => {
                        Indirection::Reference(arena.region(child(region)))
                    }
                    _ => *kind,
                },
                mutability: *mutability,
                inner: child(inner),
            },
        })
    }
}

pub(super) fn apply(info: &mut TypeInfo, slot: Slot, types: Vec<TypeId>) {
    match slot {
        Slot::Parameters => info.parameter_type_ids = Some(types),
        Slot::Field => info.field_type_id = types.first().copied(),
        Slot::Return => info.return_type_id = types.first().copied(),
        Slot::Receiver => info.receiver_type_id = types.first().copied(),
        Slot::Alias => {
            info.lexical_alias = types.first().map(|&ty| {
                super::contract::generic_return::GenericReturn::bound(
                    info.generic_param_ids.clone(),
                    info.generic_param_default_ids.clone(),
                    ty,
                )
            })
        }
    }
}

#[cfg(test)]
#[path = "module_type_inputs_tests.rs"]
mod tests;
