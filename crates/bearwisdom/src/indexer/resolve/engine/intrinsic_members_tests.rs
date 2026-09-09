use super::*;

#[test]
fn competing_or_missing_global_wrapper_owners_are_not_first_wins() {
    assert_eq!(agreed(&[]), None);
    assert_eq!(agreed(&[Some(1), None]), None);
    assert_eq!(agreed(&[Some(1), Some(2)]), None);
    assert_eq!(agreed(&[Some(1), Some(1)]), Some(1));
}

#[test]
fn intrinsic_member_projection_never_formats_a_missing_type_as_a_nominal() {
    let lookup = super::super::super::testkit::Lookup::new();
    let arena = TypeArena::new();
    for ty in [
        Type::Intrinsic(Intrinsic::Unknown),
        Type::Intrinsic(Intrinsic::String),
        Type::Literal(LitValue::Str("value".into())),
    ] {
        assert_eq!(project(&lookup, &arena, arena.intern(ty)), None);
    }
    assert!(arena.class_lookup("String").is_none());
}
