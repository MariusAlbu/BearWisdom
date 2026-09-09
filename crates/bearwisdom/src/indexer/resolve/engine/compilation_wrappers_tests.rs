use super::*;

#[test]
fn return_agreement_uses_type_ids_and_preserves_unknown_paths() {
    let arena = TypeArena::new();
    let a = arena.decl("SameName", 1);
    let b = arena.decl("SameName", 2);
    assert_eq!(agreed_return(&arena, &[Some(a), Some(a)]), a);
    for candidates in [vec![Some(a), Some(b)], vec![Some(a), None], vec![]] {
        assert_eq!(arena.get(agreed_return(&arena, &candidates)), Type::Unknown);
    }
}
