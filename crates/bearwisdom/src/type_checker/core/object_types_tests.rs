use super::super::{GenericParamData, GenericParamKind, Intrinsic, LitValue, Type};
use super::*;

#[test]
fn source_objects_keep_origin_and_nested_operands_across_substitution_and_restore() {
    let arena = TypeArena::new();
    let context = NominalContextId::fresh();
    let other = NominalContextId::fresh();
    let span = SourceSpan { start: 10, end: 50 };
    let origin = ObjectOrigin::new(context, 3, span);
    let key = arena.intern(Type::Literal(LitValue::Str("value".into())));
    let parameter = arena.intern_generic(GenericParamData {
        name: "T".into(),
        kind: GenericParamKind::Type,
        owner_symbol_index: 0,
        bound: None,
    });
    let object = SourceObject {
        origin,
        properties: vec![TypeProperty {
            key,
            value: arena.generic_type(parameter),
            optional: false,
            readonly: true,
            index: false,
        }],
    };
    let id = arena.intern(Type::Object(Box::new(object.clone())));
    assert!(arena.accepts_nominal_context(id, Some(context)));
    assert!(!arena.accepts_nominal_context(id, Some(other)));
    assert!(!arena.accepts_nominal_context(id, None));
    for origin in [
        ObjectOrigin::new(context, 4, span),
        ObjectOrigin::new(context, 3, SourceSpan { start: 11, end: 50 }),
    ] {
        assert_ne!(
            id,
            arena.intern(Type::Object(Box::new(SourceObject {
                origin,
                ..object.clone()
            })))
        );
    }
    let string = arena.intern(Type::Intrinsic(Intrinsic::String));
    let changed = crate::indexer::resolve::engine::contract::generic_return::substitute(
        &arena,
        id,
        &[(parameter, string)].into_iter().collect(),
    );
    let Type::Object(changed_object) = arena.get(changed) else {
        panic!()
    };
    assert_eq!(changed_object.origin, origin);
    assert_eq!(changed_object.properties[0].value, string);
    assert!(changed_object.properties[0].readonly);
    let foreign = arena.intern(Type::Object(Box::new(SourceObject {
        origin: ObjectOrigin::new(other, 1, span),
        properties: vec![],
    })));
    let mixed = arena.intern(Type::Object(Box::new(object.map(|_| foreign))));
    assert!(!arena.accepts_nominal_context(mixed, Some(context)));
    assert!(!arena.accepts_nominal_context(mixed, Some(other)));
    let restored = TypeArena::new();
    restored.restore_snapshot(&arena.serialize_snapshot());
    assert!(!restored.accepts_nominal_context(changed, Some(context)));
    let Type::Object(cold) = restored.get(changed) else {
        panic!()
    };
    assert_eq!(cold.origin.span, span);
    assert_eq!(cold.origin.source(), 3);
    let destination = TypeArena::new();
    destination.intern(Type::Literal(LitValue::Int(123)));
    let remap = crate::type_checker::core::arena_merge::merge_snapshot_into(
        &arena.serialize_snapshot(),
        &destination,
        &|id| id,
    )
    .unwrap();
    let Type::Object(merged) = destination.get(remap.type_id(changed).unwrap()) else {
        panic!()
    };
    assert_ne!(merged.origin.context, context);
    assert_eq!(
        destination.get(merged.properties[0].value),
        Type::Intrinsic(Intrinsic::String)
    );
    assert_eq!(
        destination.get(merged.properties[0].key),
        Type::Literal(LitValue::Str("value".into()))
    );
}
