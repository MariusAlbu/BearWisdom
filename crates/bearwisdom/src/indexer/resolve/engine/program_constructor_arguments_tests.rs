use super::*;

#[test]
fn widening_changes_literal_values_without_changing_nominal_identity() {
    let arena = TypeArena::new();
    let literal = arena.intern(Type::Literal(LitValue::Bool(true)));
    assert_eq!(
        arena.get(widen(&arena, literal)),
        Type::Intrinsic(Intrinsic::Boolean)
    );
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(widen(&arena, unknown), unknown);
}
