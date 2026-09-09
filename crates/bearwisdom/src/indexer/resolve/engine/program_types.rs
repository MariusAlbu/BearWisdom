//! Durable source-owned signatures for configured environments. No nominal or
//! generic TypeId is borrowed from the workspace during reconstruction.
use super::{
    contract::{SymbolLookup, TypeInfo},
    lexical_type_ids::TypeBinder,
    module_type_inputs::Slot,
};
use crate::indexer::{
    lexical::{type_syntax::TypeExpr, NameId},
    symbol_ids::SymbolIds,
};
use crate::type_checker::core::types::{GenericParamKind, PrimKind, Type, TypeArena, TypeId};
use crate::types::ParsedFile;
use serde::{Deserialize, Serialize};

#[path = "program_bases.rs"]
pub(super) mod bases;

#[path = "program_interface_inputs.rs"]
pub(super) mod interfaces;

#[path = "program_signature_types.rs"]
pub(super) mod source_signatures;

#[path = "program_initializer_inputs.rs"]
pub(super) mod initializers;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum Recipe {
    Unknown,
    Primitive(PrimKind),
    Declaration(Vec<i64>),
    Import(usize),
    Global(NameId),
    Intrinsic(crate::type_checker::core::types::Intrinsic),
    Literal(crate::type_checker::core::types::LitValue),
    UniqueSymbol(crate::types::SourceSpan),
    ValueQuery(crate::types::SourceSpan),
    Operator(Box<crate::type_checker::core::types::TypeOperator<Self>>),
    Parameter {
        owner: i64,
        index: usize,
    },
    SignatureParameter {
        owner: source_signatures::SignatureId,
        index: usize,
    },
    Apply(Box<Self>, Vec<Self>),
    Function(Vec<Self>, Box<Self>),
    Callable(Box<crate::type_checker::core::types::Callable<Self, crate::types::SourceSpan>>),
    Tuple(Vec<Self>),
    Union(Vec<Self>),
    Intersection(Vec<Self>),
    Optional(Box<Self>),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Signature {
    pub declaration: i64,
    pub slot: Slot,
    pub recipes: Vec<Recipe>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct Input {
    #[serde(default)]
    pub compiler_intrinsics: Vec<(
        i64,
        crate::indexer::lexical::type_syntax::compiler_intrinsics::Role,
    )>,
    #[serde(default)]
    pub initializers: Vec<initializers::Input>,
    #[serde(default)]
    pub objects: Vec<initializers::Object>,
    #[serde(default)]
    pub classes: Vec<initializers::Class>,
    #[serde(default)]
    pub computed: super::program_view::computed_keys::Input,
    #[serde(default)]
    pub unique_symbols: Vec<crate::types::SourceSpan>,
    #[serde(default)]
    pub intrinsic_members: Vec<(crate::type_checker::core::types::Intrinsic, NameId)>,
    #[serde(default)]
    pub array_types: Vec<(crate::indexer::lexical::type_syntax::arrays::Kind, NameId)>,
    #[serde(default)]
    pub source_signatures: Vec<source_signatures::Input>,
    #[serde(default)]
    pub bases: Vec<bases::Input>,
    #[serde(default)]
    pub interfaces: Vec<interfaces::Input>,
    #[serde(default)]
    pub interface_groups: Vec<Vec<i64>>,
    pub names: Vec<(NameId, String)>,
    pub declarations: Vec<i64>,
    pub parameters: Vec<(i64, Vec<(String, GenericParamKind)>)>,
    pub signatures: Vec<Signature>,
}

pub(super) fn capture(
    file: &ParsedFile,
    ids: &SymbolIds,
    lookup: &super::compilation::Compilation,
    arena: &TypeArena,
) -> Input {
    let Some(graph) = &file.flow.lexical else {
        return Input::default();
    };
    let binder = TypeBinder {
        graph,
        ids,
        path: &file.path,
        lookup,
        source: Some(lookup),
        arena,
    };
    let names: std::collections::HashMap<_, _> = graph.interned_names().collect();
    let mut input = Input {
        compiler_intrinsics: graph
            .types
            .compiler_intrinsics
            .iter()
            .filter_map(|(&slot, &role)| ids.row_id(&file.path, slot).map(|row| (row, role)))
            .collect(),
        initializers: initializers::capture(&binder),
        classes: initializers::classes(&binder),
        objects: initializers::objects(&binder),
        computed: super::program_view::computed_keys::capture(file, &binder),
        unique_symbols: graph.types.unique_symbols.clone(),
        intrinsic_members: graph
            .types
            .intrinsic_members
            .iter()
            .map(|(&kind, &name)| (kind, name))
            .collect(),
        array_types: graph
            .types
            .array_types
            .iter()
            .map(|(&kind, &name)| (kind, name))
            .collect(),
        names: names
            .iter()
            .map(|(&id, name)| (id, (*name).into()))
            .collect(),
        declarations: (0..file.symbols.len())
            .filter_map(|slot| ids.row_id(&file.path, slot))
            .collect(),
        parameters: graph
            .types
            .generic_declarations
            .iter()
            .filter_map(|(&slot, params)| {
                Some((
                    ids.row_id(&file.path, slot)?,
                    params
                        .iter()
                        .map(|(name, kind)| (names[name].into(), *kind))
                        .collect(),
                ))
            })
            .collect(),
        signatures: vec![],
        bases: bases::capture(file, ids, &binder),
        source_signatures: source_signatures::capture(&binder),
        interfaces: interfaces::capture(&binder),
        interface_groups: interfaces::groups(&binder),
    };
    for (slot, recipes) in [
        (Slot::Field, &graph.types.fields),
        (Slot::Return, &graph.types.returns),
        (Slot::Alias, &graph.types.aliases),
        (Slot::Receiver, &graph.types.receivers),
    ] {
        for (&row, recipe) in recipes {
            if let Some(declaration) = ids.row_id(&file.path, row) {
                input.signatures.push(Signature {
                    declaration,
                    slot,
                    recipes: vec![lower(recipe, &binder)],
                });
            }
        }
    }
    for (&row, recipes) in &graph.types.parameters {
        if let Some(declaration) = ids.row_id(&file.path, row) {
            input.signatures.push(Signature {
                declaration,
                slot: Slot::Parameters,
                recipes: recipes.iter().map(|r| lower(r, &binder)).collect(),
            });
        }
    }
    input
        .intrinsic_members
        .sort_unstable_by_key(|(kind, _)| *kind);
    input
        .compiler_intrinsics
        .sort_unstable_by_key(|(row, _)| *row);
    input.array_types.sort_unstable_by_key(|(kind, _)| *kind);
    input.parameters.sort_unstable_by_key(|(owner, _)| *owner);
    // Stable persistence ordering; spelling is only a source-arena payload here.
    input.names.sort_unstable_by(|a, b| a.1.cmp(&b.1));
    input
        .signatures
        .sort_unstable_by_key(|s| (s.declaration, s.slot as u8));
    input.bases.sort_unstable_by_key(|base| base.owner);
    input
}

pub(super) fn lower(expr: &TypeExpr, binder: &TypeBinder) -> Recipe {
    let child = |expr| lower(expr, binder);
    match expr {
        TypeExpr::Primitive(kind) => Recipe::Primitive(*kind),
        TypeExpr::Intrinsic(kind) => Recipe::Intrinsic(*kind),
        TypeExpr::Literal(value) => Recipe::Literal(value.clone()),
        TypeExpr::UniqueSymbol(site) => Recipe::UniqueSymbol(*site),
        TypeExpr::ValueQuery { site, .. } => Recipe::ValueQuery(*site),
        TypeExpr::Operator(op) => Recipe::Operator(Box::new(op.map(child))),
        TypeExpr::Global { name, .. } => Recipe::Global(*name),
        TypeExpr::Declaration(binding) if binder.graph.module.imports.contains_key(binding) => {
            Recipe::Import(binding.0)
        }
        TypeExpr::Declaration(binding) => {
            let rows = if let Some(slots) = binder.graph.type_symbol_slots.get(binding) {
                slots
                    .iter()
                    .map(|&slot| binder.ids.row_id(binder.path, slot))
                    .collect::<Option<Vec<_>>>()
            } else {
                binder
                    .graph
                    .symbol_slots
                    .get(binding)
                    .copied()
                    .flatten()
                    .and_then(|slot| binder.ids.row_id(binder.path, slot))
                    .map(|id| vec![id])
            };
            rows.map(Recipe::Declaration).unwrap_or(Recipe::Unknown)
        }
        TypeExpr::Parameter { owner, index } => owner
            .and_then(|slot| binder.ids.row_id(binder.path, slot))
            .map(|owner| Recipe::Parameter {
                owner,
                index: *index,
            })
            .unwrap_or(Recipe::Unknown),
        TypeExpr::SignatureParameter { owner, index } => Recipe::SignatureParameter {
            owner: *owner,
            index: *index,
        },
        TypeExpr::Apply(base, args) => {
            Recipe::Apply(Box::new(child(base)), args.iter().map(child).collect())
        }
        TypeExpr::Function(args, ret) => {
            Recipe::Function(args.iter().map(child).collect(), Box::new(child(ret)))
        }
        TypeExpr::Callable(c, _) => Recipe::Callable(Box::new(c.map(c.origin, child))),
        TypeExpr::Tuple(items) => Recipe::Tuple(items.iter().map(child).collect()),
        TypeExpr::Union(items) => Recipe::Union(items.iter().map(child).collect()),
        TypeExpr::Intersection(items) => Recipe::Intersection(items.iter().map(child).collect()),
        TypeExpr::Optional(inner) => Recipe::Optional(Box::new(child(inner))),
        // Unsupported source shapes cannot be reinterpreted as workspace names.
        _ => Recipe::Unknown,
    }
}

impl Recipe {
    pub(super) fn materialize(
        &self,
        lookup: &dyn SymbolLookup,
        arena: &TypeArena,
        path: &str,
    ) -> TypeId {
        self.materialize_with_query(lookup, arena, path, &|site| {
            lookup.source_value_type(site).flatten()
        })
    }
    pub(super) fn materialize_with_query(
        &self,
        lookup: &dyn SymbolLookup,
        arena: &TypeArena,
        path: &str,
        query: &dyn Fn(crate::types::SourceSpan) -> Option<TypeId>,
    ) -> TypeId {
        let child = |r: &Self| r.materialize_with_query(lookup, arena, path, query);
        let nominal = |row| {
            lookup
                .declaration_type(arena, row)
                .unwrap_or_else(|| arena.intern(Type::Unknown))
        };
        arena.intern(match self {
            Self::Unknown => Type::Unknown,
            Self::Primitive(kind) => Type::Primitive(*kind),
            Self::Intrinsic(kind) => Type::Intrinsic(*kind),
            Self::Literal(value) => Type::Literal(value.clone()),
            Self::UniqueSymbol(site) => {
                return lookup
                    .source_unique_symbol(*site)
                    .unwrap_or_else(|| arena.intern(Type::Unknown))
            }
            Self::ValueQuery(site) => {
                return query(*site).unwrap_or_else(|| arena.intern(Type::Unknown))
            }
            Self::Operator(op) => Type::Operator(op.map(child)),
            Self::Declaration(rows) => {
                return super::lexical_type_ids::agreed_declaration(rows.iter().copied(), lookup)
                    .map(nominal)
                    .unwrap_or_else(|| arena.intern(Type::Unknown))
            }
            Self::Import(binding) => {
                return lookup
                    .bound_import(path, crate::indexer::lexical::BindingId(*binding), true)
                    .map(nominal)
                    .unwrap_or_else(|| arena.intern(Type::Unknown))
            }
            Self::Global(name) => {
                return lookup
                    .source_global_type(*name)
                    .flatten()
                    .map(nominal)
                    .unwrap_or_else(|| arena.intern(Type::Unknown))
            }
            Self::Parameter { owner, index } => {
                return lookup
                    .canonical_type_info(lookup.canonical_decl_id(*owner))
                    .and_then(|info| info.generic_param_ids.get(*index))
                    .map(|&p| arena.generic_type(p))
                    .unwrap_or_else(|| arena.intern(Type::Unknown))
            }
            Self::SignatureParameter { owner, index } => {
                return lookup
                    .source_signature_parameter(owner.0, *index)
                    .map(|p| arena.generic_type(p))
                    .unwrap_or_else(|| arena.intern(Type::Unknown))
            }
            Self::Apply(base, args) => Type::Apply {
                base: child(base),
                args: args.iter().map(child).collect(),
            },
            Self::Function(params, ret) => Type::Function {
                params: params.iter().map(child).collect(),
                return_: child(ret),
            },
            Self::Callable(c) => match lookup.source_callable_origin(c.origin) {
                Some(origin) => Type::Callable(Box::new(c.map(origin, child))),
                None => Type::Unknown,
            },
            Self::Tuple(items) => Type::Tuple(items.iter().map(child).collect()),
            Self::Union(items) => Type::Union(items.iter().map(child).collect()),
            Self::Intersection(items) => Type::Intersection(items.iter().map(child).collect()),
            Self::Optional(inner) => Type::Optional(child(inner)),
        })
    }
}

pub(super) fn finish(info: &mut TypeInfo) {
    info.generic_return = info
        .return_type_id
        .filter(|_| !info.generic_param_ids.is_empty())
        .map(|ty| {
            super::contract::generic_return::GenericReturn::bound(
                info.generic_param_ids.clone(),
                info.generic_param_default_ids.clone(),
                ty,
            )
        });
}

#[cfg(test)]
#[path = "program_types_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "program_structural_types_tests.rs"]
mod structural_tests;
