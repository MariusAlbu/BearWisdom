use super::*;
use crate::type_checker::core::types::{Type, TypeArena};

#[test]
fn unique_identity_is_scoped_by_program_source_and_declaration_not_display() {
    let arena = TypeArena::new();
    let context = NominalContextId::fresh();
    let other = NominalContextId::fresh();
    let span = SourceSpan { start: 12, end: 48 };
    let origin = UniqueSymbol::new(context, 0, span);
    let ty = arena.intern(Type::UniqueSymbol(origin));
    assert_eq!(ty, arena.intern(Type::UniqueSymbol(origin)));
    for other in [
        UniqueSymbol::new(context, 1, span),
        UniqueSymbol::new(other, 0, span),
        UniqueSymbol::new(context, 0, SourceSpan { start: 13, end: 49 }),
    ] {
        let other = arena.intern(Type::UniqueSymbol(other));
        assert_ne!(ty, other);
        assert_eq!(arena.format_type(ty), arena.format_type(other));
    }
    assert!(arena.accepts_nominal_context(ty, Some(context)));
    assert!(!arena.accepts_nominal_context(ty, Some(other)));
    assert!(!arena.accepts_nominal_context(ty, None));
}

#[test]
fn hydration_remints_unique_and_nominal_contexts_together() {
    let src = TypeArena::new();
    let context = NominalContextId::fresh();
    let decl = src.decl_in(context, "display", 8);
    let unique = src.intern(Type::UniqueSymbol(UniqueSymbol::new(
        context,
        3,
        SourceSpan { start: 11, end: 25 },
    )));
    let tuple = src.intern(Type::Tuple(vec![decl, unique]));
    let dst = TypeArena::new();
    dst.restore_snapshot(&src.serialize_snapshot());
    let Type::Decl {
        context: Some(restored),
        ..
    } = dst.get(decl)
    else {
        panic!()
    };
    assert_ne!(restored, context);
    assert!(dst.accepts_nominal_context(tuple, Some(restored)));
    assert!(!dst.accepts_nominal_context(unique, Some(context)));
    let merged = TypeArena::new();
    let remap = crate::type_checker::core::arena_merge::merge_snapshot_into(
        &src.serialize_snapshot(),
        &merged,
        &|row| row + 100,
    )
    .unwrap();
    let Type::Decl {
        context: Some(restored),
        symbol_id: 108,
        ..
    } = merged.get(remap.type_id(decl).unwrap())
    else {
        panic!()
    };
    assert!(merged.accepts_nominal_context(remap.type_id(tuple).unwrap(), Some(restored)));
}
