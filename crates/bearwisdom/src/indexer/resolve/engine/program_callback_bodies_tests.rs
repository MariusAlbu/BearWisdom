use super::*;
use crate::indexer::resolve::engine::{
    file_lookup::FileLookup,
    program_view::merge_proof::tests::{context, owner, parse},
};
use std::{collections::HashSet, sync::Arc};

#[test]
fn contextual_bindings_are_private_and_source_predicates_keep_parameter_slots() {
    for (body, predicate) in [("value", false), ("typeof value === 'string'", true)] {
        let source = format!("export {{}}; declare const numeric: (value: number) => unknown; declare const text: (value: string) => unknown; declare const mixed: (value: string | number) => unknown; api.pick(value => {body});");
        let arena = Arc::new(TypeArena::new());
        let files = parse(&arena, &[("main.ts", &source)]);
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
        let graph = files[0].flow.lexical.as_ref().unwrap();
        let callback = graph
            .globals
            .as_ref()
            .unwrap()
            .calls
            .callbacks
            .values()
            .next()
            .unwrap();
        let lookup = FileLookup::for_file(&tree, &files[0], &ids);
        let program = tree.program_lookup("main.ts").unwrap();
        let read = *graph
            .argument_reads
            .keys()
            .max_by_key(|span| span.start)
            .unwrap();
        let before = lookup.argument_reference(read).map(|r| r.value_type);
        for (name, kind) in if predicate {
            vec![("mixed", Intrinsic::Boolean)]
        } else {
            vec![
                ("numeric", Intrinsic::Number),
                ("text", Intrinsic::String),
                ("numeric", Intrinsic::Number),
            ]
        } {
            let target = lookup
                .field_type_id_of(owner(&ids, &files[0], name))
                .unwrap();
            let ty =
                infer(&program, &lookup, graph, callback, target).expect("private candidate body");
            let Type::Callable(actual) = arena.get(ty) else {
                panic!("source callable missing");
            };
            assert_eq!(actual.origin.signature, callback.signature);
            assert_eq!(arena.get(actual.result), Type::Intrinsic(kind));
            if predicate {
                let proof = actual.predicate.unwrap();
                assert_eq!(proof.parameter, actual.parameters[0].declaration);
                assert_eq!(
                    arena.get(proof.asserted.unwrap()),
                    Type::Intrinsic(Intrinsic::String)
                );
            } else {
                assert!(actual.predicate.is_none());
            }
            assert_eq!(
                lookup.argument_reference(read).map(|r| r.value_type),
                before,
                "candidate leaked contextual facts"
            );
        }
    }
}

#[test]
fn callback_bodies_rebuild_from_portable_source_and_cold_signature_inventory() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let original = TypeArena::new();
    let source =
        "export {}; declare const context: (value: number) => unknown; api.pick(value => value);";
    let files = parse(&original, &[("main.ts", source)]);
    let payload = serde_json::to_string(&CachedParse::from_parsed(&files[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    for value in 0..40 {
        arena.intern(Type::Literal(LitValue::Int(value)));
    }
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut file = cached.into_parsed(
        &arena,
        "main.ts",
        &files[0].content_hash,
        files[0].size,
        None,
    );
    file.content = Some(source.into());
    reduce_to_contract(&mut file);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&file),
        "external",
        Some(&arena),
    )
    .unwrap();
    let row = owner(&ids, &file, "context");
    let graph = file.flow.lexical.as_ref().unwrap();
    let callback = graph
        .globals
        .as_ref()
        .unwrap()
        .calls
        .callbacks
        .values()
        .next()
        .unwrap();
    let check = |tree: &Compilation| {
        let lookup = FileLookup::for_file(tree, &file, &ids);
        let program = tree.program_lookup("main.ts").unwrap();
        let actual = infer(
            &program,
            &lookup,
            graph,
            callback,
            lookup.field_type_id_of(row).unwrap(),
        )
        .unwrap();
        let Type::Callable(actual) = tree.type_arena().unwrap().get(actual) else {
            panic!("callback origin erased");
        };
        assert_eq!(actual.origin.signature, callback.signature);
        assert_eq!(
            tree.type_arena().unwrap().get(actual.result),
            Type::Intrinsic(Intrinsic::Number)
        );
    };
    let tree = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&file])),
        &HashSet::new(),
    );
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &Default::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
}

#[test]
fn primitive_predicate_partition_preserves_literal_ids_and_unknown_barriers() {
    let arena = TypeArena::new();
    let literal = arena.intern(Type::Literal(LitValue::Str("hello".into())));
    let number = arena.intern(Type::Intrinsic(Intrinsic::Number));
    let union = arena.intern(Type::Union(vec![literal, number]));
    assert_eq!(
        narrow(&arena, union, Intrinsic::String, false, 0),
        Some(literal)
    );
    assert_eq!(
        narrow(&arena, union, Intrinsic::String, true, 0),
        Some(number)
    );
    assert_eq!(
        narrow(
            &arena,
            arena.intern(Type::Unknown),
            Intrinsic::String,
            false,
            0
        ),
        None
    );
}
