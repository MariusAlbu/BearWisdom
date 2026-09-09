//! Materialize source-bound type recipes once, before the reference loop.
use super::contract::SymbolLookup;
use crate::indexer::{
    lexical::{type_syntax::TypeExpr, LexicalBindings},
    symbol_ids::SymbolIds,
};
use crate::type_checker::core::types::{Indirection, Type, TypeArena, TypeId};

pub(super) struct TypeBinder<'a> {
    pub graph: &'a LexicalBindings,
    pub path: &'a str,
    pub ids: &'a SymbolIds,
    pub lookup: &'a dyn SymbolLookup,
    pub source: Option<&'a super::compilation::Compilation>,
    pub arena: &'a TypeArena,
}

impl TypeBinder<'_> {
    pub fn materialize(&self, recipe: &TypeExpr) -> TypeId {
        self.materialize_output(recipe, crate::type_checker::core::types::Lifetime::Unknown)
    }

    fn materialize_output(
        &self,
        recipe: &TypeExpr,
        output: crate::type_checker::core::types::Lifetime,
    ) -> TypeId {
        let lower = |ty| self.materialize_output(ty, output);
        let ty = match recipe {
            TypeExpr::Global { name, legacy } => match self.lookup.source_global_type(*name) {
                Some(id) => {
                    return id
                        .and_then(|id| self.lookup.declaration_type(self.arena, id))
                        .unwrap_or_else(|| self.arena.intern(Type::Unknown))
                }
                None => return self.arena.intern_type_str(legacy),
            },
            TypeExpr::Legacy(text) => return self.arena.intern_type_str(text),
            TypeExpr::Primitive(kind) => Type::Primitive(*kind),
            TypeExpr::Intrinsic(kind) => Type::Intrinsic(*kind),
            TypeExpr::Literal(value) => Type::Literal(value.clone()),
            TypeExpr::UniqueSymbol(site) => {
                return self
                    .lookup
                    .source_unique_symbol(*site)
                    .unwrap_or_else(|| self.arena.intern(Type::Unknown))
            }
            TypeExpr::ValueQuery { site, legacy } => {
                return match self.lookup.source_value_type(*site) {
                    Some(ty) => ty.unwrap_or_else(|| self.arena.intern(Type::Unknown)),
                    None => self.arena.intern_type_str(legacy),
                }
            }
            TypeExpr::Operator(op) => Type::Operator(op.map(lower)),
            TypeExpr::Source { usage, legacy } => {
                let fact = self
                    .source
                    .and_then(|source| source.namespace_use(self.path, *usage));
                if let Some(symbol) = fact
                    .as_ref()
                    .and_then(|f| f.declaration)
                    .and_then(|id| self.lookup.symbol_by_id(id))
                {
                    return self
                        .lookup
                        .declaration_type(self.arena, symbol.id)
                        .unwrap_or_else(|| self.arena.intern(Type::Unknown));
                }
                if self.source.is_some() && fact.is_none() && !usage.local {
                    if let Some(legacy) = legacy {
                        return self.arena.intern_type_str(legacy);
                    }
                }
                Type::Unknown
            }
            TypeExpr::Declaration(binding) => {
                let id = if self.graph.module.imports.contains_key(binding) {
                    self.lookup.bound_import(self.path, *binding, true)
                } else if let Some(slots) = self.graph.type_symbol_slots.get(binding) {
                    slots
                        .iter()
                        .map(|&slot| self.ids.row_id(self.path, slot))
                        .collect::<Option<Vec<_>>>()
                        .and_then(|rows| agreed_declaration(rows, self.lookup))
                } else {
                    self.graph
                        .symbol_slots
                        .get(binding)
                        .copied()
                        .flatten()
                        .and_then(|slot| self.ids.row_id(self.path, slot))
                };
                match id.and_then(|id| self.lookup.symbol_by_id(id)) {
                    Some(symbol) => {
                        return self
                            .lookup
                            .declaration_type(self.arena, symbol.id)
                            .unwrap_or_else(|| self.arena.intern(Type::Unknown))
                    }
                    None => Type::Unknown,
                }
            }
            TypeExpr::Parameter { owner, index } => {
                let param = owner
                    .and_then(|slot| self.ids.row_id(self.path, slot))
                    .map(|id| self.lookup.canonical_decl_id(id))
                    .and_then(|id| self.lookup.canonical_type_info(id))
                    .and_then(|info| info.generic_param_ids.get(*index).copied());
                match param {
                    Some(param) => return self.arena.generic_type(param),
                    None => Type::Unknown,
                }
            }
            TypeExpr::SignatureParameter { owner, index } => {
                match self.lookup.source_signature_parameter(owner.0, *index) {
                    Some(param) => return self.arena.generic_type(param),
                    None => Type::Unknown,
                }
            }
            TypeExpr::SourceParameter { owner, index } => {
                let param = self
                    .source
                    .and_then(|source| source.namespace_use(self.path, *owner))
                    .and_then(|f| f.declaration)
                    .and_then(|id| {
                        self.lookup
                            .canonical_type_info(self.lookup.canonical_decl_id(id))
                    })
                    .and_then(|info| info.generic_param_ids.get(*index).copied());
                match param {
                    Some(param) => return self.arena.generic_type(param),
                    None => Type::Unknown,
                }
            }
            TypeExpr::Apply(base, args) => Type::Apply {
                base: lower(base),
                args: args.iter().map(lower).collect(),
            },
            TypeExpr::Output { inputs, result } => {
                let inputs: Vec<_> = inputs.iter().map(|r| self.materialize(r)).collect();
                return self.materialize_output(
                    result,
                    super::output_lifetimes::unique(self.lookup, self.arena, &inputs),
                );
            }
            TypeExpr::OutputRegion => Type::Region(output),
            TypeExpr::OutputApplication { base, args } => {
                return super::output_lifetimes::application(
                    self.lookup,
                    self.arena,
                    output,
                    lower(base),
                    args.iter().map(lower).collect(),
                )
            }
            TypeExpr::InputRegion { owner, byte } => {
                return self
                    .ids
                    .row_id(self.path, *owner)
                    .map(|owner| {
                        super::elided_inputs::region(self.lookup, self.arena, owner, *byte, 0)
                    })
                    .unwrap_or_else(|| {
                        self.arena.intern(Type::Region(
                            crate::type_checker::core::types::Lifetime::Unknown,
                        ))
                    })
            }
            TypeExpr::InputApplication {
                owner,
                byte,
                base,
                args,
            } => {
                return self
                    .ids
                    .row_id(self.path, *owner)
                    .map(|owner| {
                        super::elided_inputs::application(
                            self.lookup,
                            self.arena,
                            owner,
                            *byte,
                            lower(base),
                            args.iter().map(lower).collect(),
                        )
                    })
                    .unwrap_or_else(|| self.arena.intern(Type::Unknown))
            }
            TypeExpr::Region(region) => Type::Region(*region),
            TypeExpr::Indirect {
                kind,
                mutability,
                region,
                inner,
            } => Type::Indirect {
                kind: match (kind, region) {
                    (Indirection::Reference(_), Some(region)) => {
                        Indirection::Reference(self.arena.region(lower(region)))
                    }
                    _ => *kind,
                },
                mutability: *mutability,
                inner: lower(inner),
            },
            TypeExpr::Function(params, ret) => Type::Function {
                params: params.iter().map(lower).collect(),
                return_: lower(ret),
            },
            TypeExpr::Callable(c, legacy) => match self.lookup.source_callable_origin(c.origin) {
                Some(origin) => Type::Callable(Box::new(c.map(origin, lower))),
                None if self.lookup.nominal_context().is_none() => return lower(legacy),
                None => Type::Unknown,
            },
            TypeExpr::Tuple(items) => Type::Tuple(items.iter().map(lower).collect()),
            TypeExpr::Union(items) => Type::Union(items.iter().map(lower).collect()),
            TypeExpr::Intersection(items) => Type::Intersection(items.iter().map(lower).collect()),
            TypeExpr::Optional(inner) => Type::Optional(lower(inner)),
            TypeExpr::Unknown => Type::Unknown,
        };
        self.arena.intern(ty)
    }
}

/// All physical rows must attest to the same identity. This is not first-match
/// selection, and an absent row or conflicting group never triggers name lookup.
pub(super) fn agreed_declaration(
    rows: impl IntoIterator<Item = i64>,
    lookup: &dyn SymbolLookup,
) -> Option<i64> {
    let mut rows = rows.into_iter();
    let first = lookup.symbol_by_id(rows.next()?)?;
    let canonical = lookup.canonical_decl_id(first.id);
    rows.all(|id| {
        lookup
            .symbol_by_id(id)
            .is_some_and(|s| lookup.canonical_decl_id(s.id) == canonical)
    })
    .then_some(canonical)
}

#[cfg(test)]
#[path = "lexical_type_ids_tests.rs"]
mod tests;
