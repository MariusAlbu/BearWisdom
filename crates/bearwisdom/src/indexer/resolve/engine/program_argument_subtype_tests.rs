use super::*;
use crate::indexer::resolve::engine::program_view::merge_proof::tests::{context, parse};
use std::{collections::HashSet, sync::Arc};

#[test]
fn broad_subtype_targets_do_not_hide_invalid_nested_callable_evidence() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[(
            "main.ts",
            "export {}; declare const callback: ((value: number) => value is string) | undefined;",
        )],
    );
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut config = context(&[&files[0]]);
    config.programs.as_mut().unwrap()[0].callable_policy =
        Some(crate::indexer::programs::CallablePolicy {
            strict_parameters: true,
            strict_nulls: true,
            bivariant_methods: None,
        });
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    let lookup = tree.program_lookup("main.ts").unwrap();
    let row = crate::indexer::resolve::engine::program_view::merge_proof::tests::owner(
        &ids, &files[0], "callback",
    );
    let source = lookup.field_type_id_of(row).unwrap();
    let relation = Relation {
        lookup: &lookup,
        arena: &arena,
    };
    for target in [Intrinsic::Any, Intrinsic::Unknown] {
        assert_eq!(
            relation.argument_in_phase(
                source,
                arena.intern(Type::Intrinsic(target)),
                std::iter::empty(),
                ArgumentRelation::Subtype
            ),
            None
        );
    }
}

#[test]
fn any_is_assignable_but_not_a_subtype_of_number_even_inside_properties() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("main.ts", "export {}; const marker = 1;")]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0]])),
        &HashSet::new(),
    );
    let lookup = tree.program_lookup("main.ts").unwrap();
    let relation = Relation {
        lookup: &lookup,
        arena: &arena,
    };
    let number = arena.intern(Type::Intrinsic(Intrinsic::Number));
    let any = arena.intern(Type::Intrinsic(Intrinsic::Any));
    let key = arena.intern(Type::Literal(LitValue::Str("value".into())));
    let object = |value| {
        arena.intern(Type::Operator(TypeOperator::Object(vec![TypeProperty {
            key,
            value,
            optional: false,
            readonly: false,
            index: false,
        }])))
    };
    for (a, b) in [(any, number), (object(any), object(number))] {
        assert_eq!(relation.argument(a, b), Some(true));
        assert_eq!(
            relation.argument_in_phase(a, b, std::iter::empty(), ArgumentRelation::Subtype),
            Some(false)
        );
    }
    assert_eq!(
        relation.argument_in_phase(number, any, std::iter::empty(), ArgumentRelation::Subtype),
        Some(true)
    );
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(
        relation.argument_in_phase(unknown, any, std::iter::empty(), ArgumentRelation::Subtype),
        None
    );
}
