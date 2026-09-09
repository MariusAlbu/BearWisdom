use super::*;
use crate::type_checker::core::types::{LitValue, Type, TypeArena};

#[test]
fn language_intrinsics_never_share_unresolved_or_nominal_identity() {
    let arena = TypeArena::new();
    let intrinsics = [
        Intrinsic::Any,
        Intrinsic::Unknown,
        Intrinsic::Never,
        Intrinsic::Void,
        Intrinsic::Undefined,
        Intrinsic::Null,
        Intrinsic::Object,
        Intrinsic::String,
        Intrinsic::Number,
        Intrinsic::Boolean,
        Intrinsic::Symbol,
        Intrinsic::BigInt,
    ];
    let unresolved = arena.intern(Type::Unknown);
    let ids: std::collections::HashSet<_> = intrinsics
        .iter()
        .map(|&kind| {
            let id = arena.intern(Type::Intrinsic(kind));
            assert_ne!(id, unresolved);
            assert_ne!(id, arena.class(kind.display()));
            id
        })
        .collect();
    assert_eq!(ids.len(), intrinsics.len());
}

#[test]
fn atomic_values_survive_snapshot_and_portable_arena_remapping() {
    let source = TypeArena::new();
    let destination = TypeArena::new();
    let types = vec![
        Type::Intrinsic(Intrinsic::Unknown),
        Type::Intrinsic(Intrinsic::Any),
        Type::Literal(LitValue::Number(1.5f64.to_bits())),
        Type::Literal(LitValue::Utf16(vec![0xd800])),
        Type::Literal(LitValue::BigInt {
            negative: true,
            words: vec![0, 0, 1],
        }),
    ];
    let ids: Vec<_> = types.iter().cloned().map(|t| source.intern(t)).collect();
    let remap = crate::type_checker::core::arena_merge::merge_snapshot_into(
        &source.serialize_snapshot(),
        &destination,
        &|id| id,
    )
    .unwrap();
    for (id, expected) in ids.iter().zip(&types) {
        assert_eq!(destination.get(remap.type_id(*id).unwrap()), *expected);
    }
    let cold = TypeArena::new();
    cold.restore_snapshot(&source.serialize_snapshot());
    for (id, expected) in ids.into_iter().zip(types) {
        assert_eq!(cold.get(id), expected);
    }
}
