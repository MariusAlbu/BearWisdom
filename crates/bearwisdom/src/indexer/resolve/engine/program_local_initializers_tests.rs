use super::super::super::merge_proof::tests::{context, parse};
use super::*;
use crate::indexer::resolve::engine::{
    contract::{FileContext, FlowCacheLookup},
    file_lookup::FileLookup,
    semantic_model::{SemanticModel, SolveOutcome},
    testkit,
};
use crate::indexer::{external_parse_payload::CachedParse, programs::CallablePolicy};
use std::{collections::HashSet, sync::Arc};

#[test]
fn configured_local_constructor_cascade_matches_compiler_portable_poisoned_and_cold() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/local_initializer_fixtures.json"
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
                arena.intern(Type::Literal(
                    crate::type_checker::core::types::LitValue::Int(n),
                ));
            }
            let cached: CachedParse = serde_json::from_str(&payload).unwrap();
            let mut file =
                cached.into_parsed(&arena, "main.ts", &parsed.content_hash, parsed.size, None);
            file.content = Some(source.clone());
            crate::indexer::contract_bindings::restore(&mut file);
            let db = crate::Database::open_in_memory().unwrap();
            let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
                &db,
                std::slice::from_ref(&file),
                "internal",
                Some(&arena),
            )
            .unwrap();
            let mut expected_calls = FxHashMap::default();
            for label in case["calls"].as_array().into_iter().flatten() {
                let expression = label["expression"].as_str().unwrap();
                let selector =
                    source.find(expression).unwrap() + expression.rfind('.').unwrap() + 1;
                let declaration = source.find(label["declaration"].as_str().unwrap()).unwrap();
                let slot = file
                    .symbols
                    .iter()
                    .position(|s| s.start_line == 0 && s.start_col as usize == declaration)
                    .unwrap();
                expected_calls.insert(selector as u32, ids.row_id("main.ts", slot).unwrap());
            }
            for symbol in &mut file.symbols {
                symbol.name = "poisoned".into();
                symbol.qualified_name = "poisoned.display".into();
                symbol.signature = None;
            }
            for reference in &mut file.refs {
                reference.call_args = vec![crate::types::CallArg::Ident("poisoned".into())];
                for segment in reference.chain.iter_mut().flat_map(|c| &mut c.segments) {
                    segment.name = "poisoned".into();
                    segment.call_args.clear();
                }
            }
            let mut config = context(&[&file]);
            config.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
                strict_parameters: true,
                strict_nulls: true,
                bivariant_methods: Some(true),
            });
            let check = |tree: &Compilation| {
                let lookup = FileLookup::for_file(tree, &file, &ids);
                if let Some(checks) = case["checks"].as_array() {
                    for label in checks {
                        let marker = label["at"].as_str().unwrap();
                        lookup.set_cursor((source.find(marker).unwrap() + marker.len()) as u32);
                        let actual = lookup.local_type_id(label["name"].as_str().unwrap());
                        if let Some(expected) = label["expected"].as_str() {
                            let expected = lookup
                                .local_type_id(expected)
                                .expect("source expected parameter");
                            assert_eq!(actual, Some(expected), "{}: {marker}", case["name"]);
                        } else {
                            assert!(
                                actual.is_none_or(
                                    |ty| tree.type_arena().unwrap().get(ty) == Type::Unknown
                                ),
                                "{}: {marker}",
                                case["name"]
                            );
                        }
                    }
                    return;
                }
                lookup.set_cursor(source.find("values.filter").unwrap() as u32);
                let actual = lookup
                    .local_type_id("excludeSet")
                    .expect("configured constructor result must reach its local BindingId");
                assert_eq!(
                    Some(actual),
                    lookup.local_type_id("expected"),
                    "{}",
                    case["name"]
                );
                let input = file
                    .flow
                    .lexical
                    .as_ref()
                    .unwrap()
                    .types
                    .initializers
                    .iter()
                    .find(|i| i.signature.0.start == source.find("excludeSet =").unwrap() as u32)
                    .unwrap();
                let target = input.target.unwrap();
                assert_eq!(
                    lookup.source_initializer_type(input.signature.0, target),
                    Some(Some(actual))
                );
                assert_eq!(
                    lookup.source_initializer_type(
                        input.signature.0,
                        crate::types::SourceSpan {
                            start: target.start + 1,
                            end: target.end
                        }
                    ),
                    Some(None)
                );
                assert_eq!(
                    lookup.source_initializer_type(
                        crate::types::SourceSpan {
                            start: input.signature.0.start,
                            end: input.signature.0.end + 1
                        },
                        target
                    ),
                    Some(None)
                );
                let ctx = FileContext {
                    file_path: "main.ts".into(),
                    language: "typescript".into(),
                    imports: vec![],
                    file_namespace: None,
                };
                let model = SemanticModel::production();
                let mut seen = FxHashMap::default();
                for (index, reference) in file
                    .refs
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.kind == crate::types::EdgeKind::Calls)
                {
                    let selector = reference
                        .chain
                        .as_ref()
                        .unwrap()
                        .segments
                        .last()
                        .unwrap()
                        .byte_offset;
                    let mut site = testkit::ref_ctx(
                        reference,
                        &file.symbols[reference.source_symbol_index],
                        vec![],
                    );
                    site.source_symbol_id = ids.row_id("main.ts", reference.source_symbol_index);
                    lookup.set_cursor(reference.byte_offset);
                    let result = model.get_symbol_info(
                        &site,
                        &ctx,
                        &lookup,
                        &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
                    );
                    let SolveOutcome::Resolved(info) = result else {
                        panic!(
                            "{}: local constructor cascade stopped at {selector}",
                            case["name"]
                        );
                    };
                    if let Some(ty) = info.resolved_yield_type {
                        lookup.record_rhs_type(index, "poisoned", ty);
                    }
                    seen.insert(selector, info.target_symbol_id);
                }
                assert_eq!(
                    seen, expected_calls,
                    "{}: exact source targets",
                    case["name"]
                );
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
fn local_initializer_results_follow_selected_provider_edits_and_deletion() {
    use super::super::super::merge_proof::tests::owner;
    use crate::indexer::symbol_ids::SymbolIds;
    let arena = Arc::new(TypeArena::new());
    let provider = "interface Payload { touch(): void } interface Collection<T> { has(item: T): boolean } interface Vector<T> { item: T } interface Factory { new<U>(items: Vector<U>): Collection<U> } declare const Build: Factory;";
    let consumer = "export {}; function run(values: Vector<Payload>) { const excludeSet = new Build(values); excludeSet.has(values.item); }";
    let files = parse(
        &arena,
        &[
            ("left.d.ts", provider),
            ("right.d.ts", provider),
            ("main.ts", consumer),
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
    let left = owner(&ids, &files[0], "Collection");
    let right = owner(&ids, &files[1], "Collection");
    let selected = |provider: &crate::types::ParsedFile, consumer: &crate::types::ParsedFile| {
        let mut config = context(&[provider, consumer]);
        config.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
            strict_parameters: true,
            strict_nulls: true,
            bivariant_methods: Some(true),
        });
        config
    };
    let check = |tree: &Compilation,
                 file: &crate::types::ParsedFile,
                 ids: &SymbolIds,
                 expected: Option<i64>| {
        let lookup = FileLookup::for_file(tree, file, ids);
        lookup.set_cursor(
            file.content
                .as_ref()
                .unwrap()
                .find("excludeSet.has")
                .unwrap() as u32,
        );
        let actual = lookup.local_type_id("excludeSet");
        assert_eq!(
            actual.and_then(|ty| head_decl_id(tree.type_arena().unwrap(), ty)),
            expected
        );
        actual
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&selected(&files[0], &files[2])),
        &HashSet::new(),
    );
    let old = check(&tree, &files[2], &ids, Some(left)).unwrap();
    tree.persist_type_info(db.conn()).unwrap();
    let mut changed = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&arena),
        Some(&selected(&files[1], &files[2])),
        &HashSet::new(),
    );
    changed.ingest_from_db(db.conn());
    check(&changed, &files[2], &ids, Some(right));
    assert!(
        !(&changed.program_lookup("main.ts").unwrap() as &dyn SymbolLookup)
            .accepts_type_context(&arena, old)
    );
    changed.persist_type_info(db.conn()).unwrap();
    let invalid = parse(
        &arena,
        &[(
            "right.d.ts",
            &provider.replace("items: Vector<U>", "items: string"),
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
        Some(&selected(&invalid[0], &files[2])),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, &files[2], &ids, None);
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, &files[2], &ids, None);
    db.conn()
        .execute("DELETE FROM files WHERE path='right.d.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    deleted.ingest_from_db(db.conn());
    check(&deleted, &files[2], &ids, None);
    let mut recovered = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&restored),
        Some(&selected(&files[0], &files[2])),
        &HashSet::new(),
    );
    recovered.ingest_from_db(db.conn());
    check(&recovered, &files[2], &ids, Some(left));
    recovered.persist_type_info(db.conn()).unwrap();
    let consumer_edit = parse(
        &restored,
        &[(
            "main.ts",
            &consumer.replace("new Build(values)", "new Build('invalid')"),
        )],
    );
    let (_, consumer_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &consumer_edit,
        "internal",
        Some(&restored),
    )
    .unwrap();
    let mut edited = Compilation::build_with_context(
        &consumer_edit,
        &consumer_ids,
        Arc::clone(&restored),
        Some(&selected(&files[0], &consumer_edit[0])),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, &consumer_edit[0], &consumer_ids, None);
    check(&edited, &files[2], &ids, None);
    edited.persist_type_info(db.conn()).unwrap();
    let final_arena = Arc::new(TypeArena::new());
    final_arena.restore_snapshot(&restored.serialize_snapshot());
    let mut final_cold = Compilation::build(&[], &SymbolIds::default(), final_arena);
    final_cold.ingest_from_db(db.conn());
    check(&final_cold, &consumer_edit[0], &consumer_ids, None);
    check(&final_cold, &files[2], &ids, None);
}
