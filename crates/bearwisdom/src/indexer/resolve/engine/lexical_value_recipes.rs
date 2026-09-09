//! Source expression evaluation uses only binding IDs, positions and owner IDs.
use crate::indexer::lexical::{type_syntax::ValueExpr, BindingId};
use crate::type_checker::core::types::{Indirection, Lifetime, Type, TypeArena, TypeId};

#[path = "value_projection.rs"]
mod projection;
use crate::indexer::resolve::engine::member_index::MemberNameId;
pub(super) use projection::Context;
pub(super) type BoundValue = ValueExpr<i64, MemberNameId>;

pub(super) fn lower(
    expr: &ValueExpr,
    owner: &impl Fn(usize) -> Option<i64>,
    name: &impl Fn(crate::indexer::lexical::NameId) -> Option<MemberNameId>,
    depth: usize,
) -> BoundValue {
    if depth >= 32 {
        return ValueExpr::Unknown;
    }
    match expr {
        ValueExpr::Read { binding, byte } => ValueExpr::Read {
            binding: *binding,
            byte: *byte,
        },
        ValueExpr::VariantField {
            variant,
            field,
            byte,
            operand,
        } => name(*variant)
            .zip(name(*field))
            .map(|(variant, field)| ValueExpr::VariantField {
                variant,
                field,
                byte: *byte,
                operand: Box::new(lower(operand, owner, name, depth + 1)),
            })
            .unwrap_or(ValueExpr::Unknown),
        ValueExpr::Borrow {
            owner: slot,
            span,
            mutability,
            operand,
        } => owner(*slot)
            .map(|id| ValueExpr::Borrow {
                owner: id,
                span: *span,
                mutability: *mutability,
                operand: Box::new(lower(operand, owner, name, depth + 1)),
            })
            .unwrap_or(ValueExpr::Unknown),
        ValueExpr::Field {
            name: source,
            byte,
            operand,
        } => name(*source)
            .map(|id| ValueExpr::Field {
                name: id,
                byte: *byte,
                operand: Box::new(lower(operand, owner, name, depth + 1)),
            })
            .unwrap_or(ValueExpr::Unknown),
        ValueExpr::TupleIndex { index, operand } => ValueExpr::TupleIndex {
            index: *index,
            operand: Box::new(lower(operand, owner, name, depth + 1)),
        },
        ValueExpr::Dereference { operand } => ValueExpr::Dereference {
            operand: Box::new(lower(operand, owner, name, depth + 1)),
        },
        ValueExpr::Unknown => ValueExpr::Unknown,
    }
}

pub(super) fn evaluate(
    expr: &BoundValue,
    arena: &TypeArena,
    depth: usize,
    context: Option<&Context>,
    read: &impl Fn(BindingId, u32, usize) -> Option<TypeId>,
) -> Option<TypeId> {
    let unknown = arena.intern(Type::Unknown);
    if depth >= 128 {
        return Some(unknown);
    }
    match expr {
        ValueExpr::Read { binding, byte } => read(*binding, *byte, depth + 1),
        ValueExpr::VariantField {
            variant,
            field,
            byte,
            operand,
        } => Some(
            context
                .and_then(|c| {
                    let ty = evaluate(operand, arena, depth + 1, context, read)?;
                    c.variant_field(arena, ty, *variant, *field, *byte)
                })
                .unwrap_or(unknown),
        ),
        ValueExpr::Borrow {
            owner,
            span,
            mutability,
            operand,
        } => {
            let inner = evaluate(operand, arena, depth + 1, context, read).unwrap_or(unknown);
            if matches!(arena.get(inner), Type::Unknown | Type::Class(_)) {
                return Some(unknown);
            }
            Some(arena.intern(Type::Indirect {
                kind: Indirection::Reference(Lifetime::Inference {
                    owner: *owner,
                    byte: span.start,
                }),
                mutability: *mutability,
                inner,
            }))
        }
        ValueExpr::Field {
            name,
            byte,
            operand,
        } => Some(
            context
                .and_then(|c| {
                    let ty = evaluate(operand, arena, depth + 1, context, read)?;
                    c.field(arena, ty, *name, *byte)
                })
                .unwrap_or(unknown),
        ),
        ValueExpr::TupleIndex { index, operand } => Some(
            context
                .and_then(|c| {
                    let ty = c.field_receiver(
                        arena,
                        evaluate(operand, arena, depth + 1, context, read)?,
                    )?;
                    match arena.get(ty) {
                        Type::Tuple(items) => items.get(*index).copied(),
                        _ => None,
                    }
                })
                .unwrap_or(unknown),
        ),
        ValueExpr::Dereference { operand } => Some(
            context
                .and_then(|c| {
                    let ty =
                        c.expand(arena, evaluate(operand, arena, depth + 1, context, read)?)?;
                    match arena.get(ty) {
                        Type::Indirect { inner, .. } => Some(inner),
                        _ => None,
                    }
                })
                .unwrap_or(unknown),
        ),
        ValueExpr::Unknown => Some(unknown),
    }
}

/// Fill only unknown reference regions in an otherwise exactly corresponding
/// annotation. Unknown pointees and namesakes are never equality evidence.
pub(super) fn refine_regions(
    arena: &TypeArena,
    declared: TypeId,
    actual: TypeId,
    depth: usize,
) -> Option<TypeId> {
    if depth >= 32 {
        return None;
    }
    match (arena.get(declared), arena.get(actual)) {
        (Type::Unknown | Type::Class(_), _) | (_, Type::Unknown | Type::Class(_)) => None,
        (
            Type::Indirect {
                kind: Indirection::Reference(d),
                mutability: dm,
                inner: di,
            },
            Type::Indirect {
                kind: Indirection::Reference(a),
                mutability: am,
                inner: ai,
            },
        ) if dm == am => {
            if d != Lifetime::Unknown && d != a {
                return None;
            }
            let inner = refine_regions(arena, di, ai, depth + 1)?;
            Some(arena.intern(Type::Indirect {
                kind: Indirection::Reference(if d == Lifetime::Unknown { a } else { d }),
                mutability: dm,
                inner,
            }))
        }
        (
            Type::Decl {
                symbol_id: d,
                context: dc,
                ..
            },
            Type::Decl {
                symbol_id: a,
                context: ac,
                ..
            },
        ) if (dc, d) == (ac, a) => Some(declared),
        _ if declared == actual => Some(declared),
        _ => None,
    }
}

#[cfg(test)]
#[path = "lexical_value_recipes_tests.rs"]
mod tests;
