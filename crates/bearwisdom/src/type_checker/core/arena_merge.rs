// =============================================================================
// type_checker/core/arena_merge — re-intern one arena's types into another
//
// Merges a serialized arena snapshot into a destination arena, returning the
// TypeId remap table. Types are processed in snapshot id order — a child's id
// is always smaller than its parent's (`restore_snapshot` invariant) — so one
// forward pass sees every child already remapped. Generic params remap lazily
// on first reference: a param's bound TypeId was interned before the param
// existed, so it is always already in the table when the param is reached.
// Context-free structural types dedup in the destination. Generic parameters
// and configured nominal contexts are fresh identities per imported snapshot;
// a physical-row remap does not attest equivalence of two program instances.
// =============================================================================

use rustc_hash::FxHashMap;

use super::types::{GenericParamId, Indirection, Lifetime, Type, TypeArena, TypeId};

/// Remap table from source-snapshot TypeIds to destination TypeIds.
/// Indexed by `source_id.index()`; sized to the snapshot's type count.
pub struct ArenaRemap {
    types: Vec<TypeId>,
}

impl ArenaRemap {
    /// Destination TypeId for a source-snapshot TypeId.
    pub fn type_id(&self, src: TypeId) -> Option<TypeId> {
        self.types.get(src.index()).copied()
    }
}

/// Merge `snapshot` (a `TypeArena::serialize_snapshot` blob) into `dst`.
/// `remap_decl_id` translates `Type::Decl` symbol ids from the snapshot's id
/// space into the destination's (identity when the spaces coincide). Returns
/// `None` when the blob does not parse.
pub fn merge_snapshot_into(
    snapshot: &str,
    dst: &TypeArena,
    remap_decl_id: &dyn Fn(i64) -> i64,
) -> Option<ArenaRemap> {
    let src = TypeArena::new();
    let n = src.restore_snapshot(snapshot);
    if n == 0 && snapshot.trim() != "[[],[]]" {
        return None;
    }

    let mut types: Vec<TypeId> = Vec::with_capacity(n);
    let mut generics: FxHashMap<GenericParamId, GenericParamId> = FxHashMap::default();

    for i in 0..n {
        let src_id =
            TypeId(std::num::NonZeroU32::new((i + 1) as u32).expect("arena index overflow"));
        let remapped = remap_type(
            src.get(src_id),
            &types,
            &mut generics,
            &src,
            dst,
            remap_decl_id,
        );
        types.push(dst.intern(remapped));
    }
    Some(ArenaRemap { types })
}

/// Rebuild one source `Type` with every child id translated to the
/// destination. Children always precede parents in the table, so a plain
/// index read suffices; a missing child indicates a corrupt snapshot and maps
/// to `Unknown` rather than panicking.
fn remap_type(
    ty: Type,
    table: &[TypeId],
    generics: &mut FxHashMap<GenericParamId, GenericParamId>,
    src: &TypeArena,
    dst: &TypeArena,
    remap_decl_id: &dyn Fn(i64) -> i64,
) -> Type {
    let map = |id: TypeId, table: &[TypeId]| table.get(id.index()).copied();
    let mut parameter = |param| {
        *generics.entry(param).or_insert_with(|| {
            let mut data = src.generic_param(param);
            data.bound = data.bound.and_then(|b| map(b, table));
            dst.intern_generic(data)
        })
    };
    match ty {
        Type::Class(_)
        | Type::Primitive(_)
        | Type::Intrinsic(_)
        | Type::UniqueSymbol(_)
        | Type::Literal(_)
        | Type::Unknown => ty,
        Type::Operator(op) => Type::Operator(
            op.map(|id| map(*id, table).unwrap_or_else(|| dst.intern(Type::Unknown))),
        ),
        Type::Callable(c) => Type::Callable(Box::new(c.map(c.origin, |id| {
            map(*id, table).unwrap_or_else(|| dst.intern(Type::Unknown))
        }))),
        Type::Object(object) => {
            Type::Object(Box::new(object.map(|id| {
                map(*id, table).unwrap_or_else(|| dst.intern(Type::Unknown))
            })))
        }
        Type::Decl {
            symbol_id,
            qname,
            context,
        } => Type::Decl {
            symbol_id: remap_decl_id(symbol_id),
            qname,
            context,
        },
        Type::Function { params, return_ } => Type::Function {
            params: params.into_iter().filter_map(|p| map(p, table)).collect(),
            return_: map(return_, table).unwrap_or_else(|| dst.intern(Type::Unknown)),
        },
        Type::Tuple(items) => {
            Type::Tuple(items.into_iter().filter_map(|t| map(t, table)).collect())
        }
        Type::Union(items) => {
            Type::Union(items.into_iter().filter_map(|t| map(t, table)).collect())
        }
        Type::Intersection(items) => {
            Type::Intersection(items.into_iter().filter_map(|t| map(t, table)).collect())
        }
        Type::Apply { base, args } => Type::Apply {
            base: map(base, table).unwrap_or_else(|| dst.intern(Type::Unknown)),
            args: args.into_iter().filter_map(|t| map(t, table)).collect(),
        },
        Type::Generic { param } => Type::Generic {
            param: parameter(param),
        },
        Type::Region(region) => Type::Region(match region {
            Lifetime::Parameter(param) => Lifetime::Parameter(parameter(param)),
            Lifetime::Inference { owner, byte } => Lifetime::Inference {
                owner: remap_decl_id(owner),
                byte,
            },
            other => other,
        }),
        Type::Optional(inner) => {
            Type::Optional(map(inner, table).unwrap_or_else(|| dst.intern(Type::Unknown)))
        }
        Type::Indirect {
            kind,
            mutability,
            inner,
        } => Type::Indirect {
            kind: match kind {
                Indirection::Reference(Lifetime::Parameter(param)) => {
                    Indirection::Reference(Lifetime::Parameter(parameter(param)))
                }
                Indirection::Reference(Lifetime::Inference { owner, byte }) => {
                    Indirection::Reference(Lifetime::Inference {
                        owner: remap_decl_id(owner),
                        byte,
                    })
                }
                other => other,
            },
            mutability,
            inner: map(inner, table).unwrap_or_else(|| dst.intern(Type::Unknown)),
        },
        Type::AsyncWrapper(inner) => {
            Type::AsyncWrapper(map(inner, table).unwrap_or_else(|| dst.intern(Type::Unknown)))
        }
        Type::Iterator(inner) => {
            Type::Iterator(map(inner, table).unwrap_or_else(|| dst.intern(Type::Unknown)))
        }
        Type::Constructor(inner) => {
            Type::Constructor(map(inner, table).unwrap_or_else(|| dst.intern(Type::Unknown)))
        }
    }
}

#[cfg(test)]
#[path = "arena_merge_tests.rs"]
mod tests;
