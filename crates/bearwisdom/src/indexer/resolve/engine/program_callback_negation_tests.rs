use super::*;
use crate::indexer::resolve::engine::{
    file_lookup::FileLookup,
    program_view::merge_proof::tests::{context, owner, parse},
};
use crate::indexer::{external_parse_payload::CachedParse, programs::CallablePolicy};
use std::{collections::HashSet, sync::Arc};

#[test]
fn callback_negation_matches_compiler_with_poisoned_portable_and_cold_evidence() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/callback_negation_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        for prefix in ["", "export {}; "] {
            let source = format!("{prefix}{}", case["source"].as_str().unwrap());
            let original = TypeArena::new();
            let parsed = parse(&original, &[("main.ts", &source)]).remove(0);
            let payload =
                serde_json::to_string(&CachedParse::from_parsed(&parsed, &original)).unwrap();
            let arena = Arc::new(TypeArena::new());
            for n in 0..32 {
                arena.intern(Type::Literal(LitValue::Int(n)));
            }
            let cached: CachedParse = serde_json::from_str(&payload).unwrap();
            let mut file =
                cached.into_parsed(&arena, "main.ts", &parsed.content_hash, parsed.size, None);
            file.content = Some(source.clone());
            crate::indexer::contract_bindings::restore(&mut file);
            for reference in &mut file.refs {
                reference.call_args = vec![crate::types::CallArg::Ident("poisoned".into())];
                for segment in reference
                    .chain
                    .iter_mut()
                    .flat_map(|chain| &mut chain.segments)
                {
                    segment.name = "poisoned".into();
                    segment.call_args.clear();
                }
            }
            let db = crate::Database::open_in_memory().unwrap();
            let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
                &db,
                std::slice::from_ref(&file),
                "internal",
                Some(&arena),
            )
            .unwrap();
            let target = owner(&ids, &file, "context");
            let expected = owner(&ids, &file, "expected");
            let api = owner(&ids, &file, "api");
            let expected_result = owner(&ids, &file, "expectedResult");
            let selector = source.find("api.pick").unwrap() as u32 + 4;
            for symbol in &mut file.symbols {
                symbol.name = "poisoned".into();
                symbol.qualified_name = "poisoned.display".into();
                symbol.signature = None;
            }
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
            let mut config = context(&[&file]);
            config.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
                strict_parameters: true,
                strict_nulls: true,
                bivariant_methods: Some(true),
            });
            let check = |tree: &Compilation| {
                let file_lookup = FileLookup::for_file(tree, &file, &ids);
                let lookup = tree.program_lookup("main.ts").unwrap();
                let arena = tree.type_arena().unwrap();
                let before: Vec<_> = graph
                    .argument_reads
                    .keys()
                    .map(|&site| {
                        (
                            site,
                            file_lookup
                                .argument_reference(site)
                                .and_then(|r| r.value_type),
                        )
                    })
                    .collect();
                let actual = infer(
                    &lookup,
                    &file_lookup,
                    graph,
                    callback,
                    lookup.field_type_id_of(target).unwrap(),
                );
                let arguments = file_lookup
                    .source_call_arguments(selector)
                    .unwrap()
                    .unwrap();
                let arguments = crate::indexer::resolve::engine::arg_types::resolve_arg_types(
                    &file_lookup,
                    arena,
                    arguments,
                );
                let selected = file_lookup
                    .overloaded_call(
                        lookup.field_type_id_of(api).unwrap(),
                        selector,
                        &arguments,
                        &[],
                    )
                    .unwrap();
                if case["supported"].as_bool().unwrap() {
                    let Type::Callable(actual) = arena.get(actual.unwrap_or_else(|| {
                        panic!("{} ({prefix}): callback incomplete", case["name"])
                    })) else {
                        panic!("callable lost");
                    };
                    assert_eq!(actual.origin.signature, callback.signature);
                    assert_eq!(
                        arena.get(actual.result),
                        Type::Intrinsic(Intrinsic::Boolean)
                    );
                    assert_eq!(
                        actual.predicate.is_some(),
                        case["predicate"].as_bool().unwrap(),
                        "{}",
                        case["name"]
                    );
                    if let Some(predicate) = actual.predicate {
                        assert_eq!(predicate.parameter, actual.parameters[0].declaration);
                        let relation = Relation {
                            lookup: &lookup,
                            arena,
                        };
                        assert_eq!(
                            relation.canonical(predicate.asserted.unwrap(), 0),
                            relation.canonical(lookup.field_type_id_of(expected).unwrap(), 0),
                            "{}",
                            case["name"]
                        );
                    }
                    let selected = selected.unwrap_or_else(|_| {
                        panic!("{}: negation overload not selected", case["name"])
                    });
                    assert_eq!(selected.origins.len(), 2);
                    let origin = &selected.origins[selected.selected];
                    assert_eq!(
                        origin.span.start,
                        source.find(case["selected"].as_str().unwrap()).unwrap() as u32,
                        "{}",
                        case["name"]
                    );
                    assert!(origin
                        .declaration
                        .and_then(|row| lookup.symbol_by_id(row))
                        .is_some());
                    let relation = Relation {
                        lookup: &lookup,
                        arena,
                    };
                    assert_eq!(
                        Some(selected.return_type),
                        relation.canonical(lookup.field_type_id_of(expected_result).unwrap(), 0),
                        "{}",
                        case["name"]
                    );
                } else {
                    assert!(
                        actual.is_none(),
                        "{}: invalid operand supplied a callback",
                        case["name"]
                    );
                    assert!(
                        selected.is_err(),
                        "{}: invalid operand selected an overload",
                        case["name"]
                    );
                }
                for (site, prior) in before {
                    assert_eq!(
                        file_lookup
                            .argument_reference(site)
                            .and_then(|r| r.value_type),
                        prior,
                        "private callback leaked"
                    );
                }
            };
            let tree = Compilation::build_with_context(
                std::slice::from_ref(&file),
                &ids,
                Arc::clone(&arena),
                Some(&config),
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
    }
}

#[test]
fn callback_negation_rebinds_provider_edits_deletion_and_foreign_contexts() {
    use crate::indexer::symbol_ids::SymbolIds;
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/callback_negation_fixtures.json"
    ))
    .unwrap();
    let source = cases
        .iter()
        .find(|c| c["name"] == "captured_negated_method_operand")
        .unwrap()["source"]
        .as_str()
        .unwrap();
    let (provider, consumer) = source.split_at(source.find("declare const context:").unwrap());
    let consumer = format!("export {{}}; {consumer}");
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            ("left.d.ts", provider),
            ("right.d.ts", provider),
            ("main.ts", &consumer),
        ],
    );
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let target = owner(&ids, &files[2], "context");
    let api = owner(&ids, &files[2], "api");
    let selected = |provider: &crate::types::ParsedFile| {
        let mut config = context(&[provider, &files[2]]);
        config.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
            strict_parameters: true,
            strict_nulls: true,
            bivariant_methods: Some(true),
        });
        config
    };
    let check = |tree: &Compilation, valid: bool, foreign: Option<TypeId>| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        let arena = tree.type_arena().unwrap();
        let file_lookup = FileLookup::for_file(tree, &files[2], &ids);
        let graph = files[2].flow.lexical.as_ref().unwrap();
        let callback = graph
            .globals
            .as_ref()
            .unwrap()
            .calls
            .callbacks
            .values()
            .next()
            .unwrap();
        let actual = lookup
            .field_type_id_of(target)
            .and_then(|target| infer(&lookup, &file_lookup, graph, callback, target));
        assert_eq!(
            actual.is_some(),
            valid,
            "negation must validate its captured operand"
        );
        if let Some(foreign) = foreign {
            assert!(infer(&lookup, &file_lookup, graph, callback, foreign).is_none());
        }
        let selector = consumer.find("api.pick").unwrap() as u32 + 4;
        let call = lookup.field_type_id_of(api).and_then(|receiver| {
            let args = file_lookup.source_call_arguments(selector)?.ok()?;
            let args = crate::indexer::resolve::engine::arg_types::resolve_arg_types(
                &file_lookup,
                arena,
                args,
            );
            file_lookup
                .overloaded_call(receiver, selector, &args, &[])?
                .ok()
        });
        assert_eq!(
            call.is_some(),
            valid,
            "stale captured operand selected an overload"
        );
        if let Some(call) = call {
            assert_eq!(
                call.origins[call.selected].span.start,
                consumer.find("pick(callback:").unwrap() as u32
            );
        }
        actual
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&selected(&files[0])),
        &HashSet::new(),
    );
    let old = check(&tree, true, None).unwrap();
    tree.persist_type_info(db.conn()).unwrap();
    let mut changed = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&arena),
        Some(&selected(&files[1])),
        &HashSet::new(),
    );
    changed.ingest_from_db(db.conn());
    check(&changed, true, Some(old));
    changed.persist_type_info(db.conn()).unwrap();
    let invalid = parse(
        &arena,
        &[(
            "right.d.ts",
            &provider.replace("ready(value: number)", "ready(value: string)"),
        )],
    );
    let (_, invalid_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &invalid,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut edited = Compilation::build_with_context(
        &invalid,
        &invalid_ids,
        Arc::clone(&arena),
        Some(&selected(&invalid[0])),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, false, Some(old));
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, false, None);
    db.conn()
        .execute("DELETE FROM files WHERE path='right.d.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    deleted.ingest_from_db(db.conn());
    check(&deleted, false, None);
    let mut recovered = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&restored),
        Some(&selected(&files[0])),
        &HashSet::new(),
    );
    recovered.ingest_from_db(db.conn());
    check(&recovered, true, None);
    recovered.persist_type_info(db.conn()).unwrap();
    let final_arena = Arc::new(TypeArena::new());
    final_arena.restore_snapshot(&restored.serialize_snapshot());
    let mut final_cold = Compilation::build(&[], &SymbolIds::default(), final_arena);
    final_cold.ingest_from_db(db.conn());
    check(&final_cold, true, None);
}
