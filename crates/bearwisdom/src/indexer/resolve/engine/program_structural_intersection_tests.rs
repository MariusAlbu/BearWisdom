use super::super::super::tests::with_relation;
use super::*;

#[test]
fn empty_object_intersection_removes_nullish_but_not_missing_evidence() {
    with_relation(|relation| {
        let arena = relation.arena;
        let empty = arena.intern(Type::Operator(TypeOperator::Object(vec![])));
        let string = arena.intern(Type::Intrinsic(Intrinsic::String));
        let null = arena.intern(Type::Intrinsic(Intrinsic::Null));
        let union = arena.intern(Type::Union(vec![null, string]));
        let intersection = arena.intern(Type::Intersection(vec![empty, union]));
        assert!(relation.equal(intersection, string));
        let missing = arena.intern(Type::Unknown);
        assert!(!relation.equal(
            arena.intern(Type::Intersection(vec![empty, missing])),
            empty
        ));
    });
}

#[test]
fn assignable_intersection_surface_is_not_declaration_merge_identity() {
    with_relation(|relation| {
        let arena = relation.arena;
        let string = arena.intern(Type::Intrinsic(Intrinsic::String));
        let property = |key: &str| TypeProperty {
            key: arena.intern(Type::Literal(LitValue::Str(key.into()))),
            value: string,
            optional: false,
            readonly: false,
            index: false,
        };
        let a = property("a");
        let b = property("b");
        let object = |properties| arena.intern(Type::Operator(TypeOperator::Object(properties)));
        let left = arena.intern(Type::Intersection(vec![
            object(vec![a.clone()]),
            object(vec![b.clone()]),
        ]));
        let right = object(vec![a, b]);
        assert!(relation.assignable(left, right));
        assert!(relation.assignable(right, left));
        assert!(!relation.equal(left, right));
    });
}
