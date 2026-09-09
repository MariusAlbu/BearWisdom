use super::super::tests::{context, owner, parse};
use super::*;
use crate::indexer::{programs::CallablePolicy, symbol_ids::SymbolIds};
use std::{collections::HashSet, sync::Arc};

#[test]
fn method_heritage_policy_changes_are_effective_without_source_edits() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("main.ts", "export {}; interface Base { read(x: string | number): string } interface Catalog extends Base { read(x: string): string }")]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let catalog = owner(&ids, &files[0], "Catalog");
    let mut prior = None;
    for policy in [Some(true), Some(false), None, Some(true)] {
        let mut config = context(&[&files[0]]);
        config.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
            strict_parameters: true,
            strict_nulls: true,
            bivariant_methods: policy,
        });
        let mut tree = if prior.is_none() {
            Compilation::build_with_context(
                &files,
                &ids,
                Arc::clone(&arena),
                Some(&config),
                &HashSet::new(),
            )
        } else {
            Compilation::build_with_context(
                &[],
                &SymbolIds::default(),
                Arc::clone(&arena),
                Some(&config),
                &HashSet::new(),
            )
        };
        tree.ingest_from_db(db.conn());
        let check = |tree: &Compilation| {
            let lookup = tree.program_lookup("main.ts").unwrap();
            assert_eq!(lookup.symbol_by_id(catalog).is_some(), policy == Some(true));
        };
        check(&tree);
        let lookup = tree.program_lookup("main.ts").unwrap();
        if let Some(old) = prior {
            assert!(!(&lookup as &dyn SymbolLookup).accepts_type_context(&arena, old));
        }
        if let Some(ty) = (&lookup as &dyn SymbolLookup).declaration_type(&arena, catalog) {
            prior = Some(ty);
        }
        tree.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
        cold.ingest_from_db(db.conn());
        check(&cold);
    }
}

#[test]
fn recursive_method_heritage_retargets_and_revokes_selected_ancestor_evidence() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            (
                "left.d.ts",
                "interface Base<T> { self(): Base<T>; read(): T }",
            ),
            (
                "right.d.ts",
                "interface Base<T> { self(): Base<T>; read(): T }",
            ),
            (
                "main.ts",
                "export {}; interface Catalog<T> extends Base<T> { self(): Catalog<T> }",
            ),
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
    let catalog = owner(&ids, &files[2], "Catalog");
    let make_config = |provider: &crate::types::ParsedFile| {
        let mut config = context(&[provider, &files[2]]);
        config.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
            strict_parameters: true,
            strict_nulls: true,
            bivariant_methods: Some(true),
        });
        config
    };
    let check = |tree: &Compilation, parent: Option<i64>, read: Option<i64>| {
        use crate::indexer::resolve::engine::member_selection::{select, Selection};
        let lookup = tree.program_lookup("main.ts").unwrap();
        assert_eq!(lookup.symbol_by_id(catalog).is_some(), parent.is_some());
        if let Some(parent) = parent {
            assert_eq!(lookup.parent_class_ids(catalog), [parent]);
            let name = lookup.member_index().unwrap().name("read").unwrap();
            assert_eq!(
                select(&lookup, catalog, name, &|_| true),
                Selection::Unique(read.unwrap())
            );
        }
    };
    let left = (
        owner(&ids, &files[0], "Base"),
        owner(&ids, &files[0], "read"),
    );
    let right = (
        owner(&ids, &files[1], "Base"),
        owner(&ids, &files[1], "read"),
    );
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&make_config(&files[0])),
        &HashSet::new(),
    );
    check(&tree, Some(left.0), Some(left.1));
    tree.persist_type_info(db.conn()).unwrap();
    let mut switched = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&arena),
        Some(&make_config(&files[1])),
        &HashSet::new(),
    );
    switched.ingest_from_db(db.conn());
    check(&switched, Some(right.0), Some(right.1));
    switched.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, Some(right.0), Some(right.1));
    let changed = parse(
        &restored,
        &[(
            "right.d.ts",
            "interface Base<T> { self(): Base<string>; read(): T }",
        )],
    );
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&restored),
    )
    .unwrap();
    let mut edited = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&restored),
        Some(&make_config(&changed[0])),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, None, None);
    edited.persist_type_info(db.conn()).unwrap();
    let next = Arc::new(TypeArena::new());
    next.restore_snapshot(&restored.serialize_snapshot());
    let mut edited_cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&next));
    edited_cold.ingest_from_db(db.conn());
    check(&edited_cold, None, None);
    assert!(edited_cold
        .source_program_lookup(&files[1])
        .unwrap()
        .symbol_by_id(right.0)
        .is_none());
    db.conn()
        .execute("DELETE FROM files WHERE path='right.d.ts'", [])
        .unwrap();
    let mut stale = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&next));
    stale.ingest_from_db(db.conn());
    check(&stale, None, None);
    let mut deleted = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&next),
        Some(&make_config(&files[0])),
        &HashSet::new(),
    );
    deleted.ingest_from_db(db.conn());
    check(&deleted, Some(left.0), Some(left.1));
    deleted.persist_type_info(db.conn()).unwrap();
    let final_arena = Arc::new(TypeArena::new());
    final_arena.restore_snapshot(&next.serialize_snapshot());
    let mut final_cold = Compilation::build(&[], &SymbolIds::default(), final_arena);
    final_cold.ingest_from_db(db.conn());
    check(&final_cold, Some(left.0), Some(left.1));
}

#[test]
fn method_heritage_cascade_keeps_exact_targets_with_poisoned_portable_and_cold_inputs() {
    use crate::indexer::external_parse_payload::CachedParse;
    use crate::indexer::resolve::engine::{
        contract::{FileContext, FlowCacheLookup},
        file_lookup::FileLookup,
        semantic_model::{SemanticModel, SolveOutcome},
        testkit,
    };
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/method_heritage_fixtures.json"
    ))
    .unwrap();
    for case in cases.iter().filter(|case| case["calls"].is_array()) {
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
            // Portable extraction payloads deliberately omit flow. Rebuild source
            // identity with the normal hydration helper while retaining call refs.
            crate::indexer::contract_bindings::restore(&mut file);
            let db = crate::Database::open_in_memory().unwrap();
            let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
                &db,
                std::slice::from_ref(&file),
                "internal",
                Some(&arena),
            )
            .unwrap();
            let mut expected = FxHashMap::default();
            for label in case["calls"].as_array().unwrap() {
                let expression = label["expression"].as_str().unwrap();
                let selector =
                    source.find(expression).unwrap() + expression.rfind('.').unwrap() + 1;
                let declaration = source.find(label["declaration"].as_str().unwrap()).unwrap();
                let slot = file
                    .symbols
                    .iter()
                    .position(|s| s.start_line == 0 && s.start_col as usize == declaration)
                    .unwrap();
                expected.insert(selector as u32, ids.row_id("main.ts", slot).unwrap());
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
            let check = |tree: &Compilation| {
                let lookup = FileLookup::for_file(tree, &file, &ids);
                let context = FileContext {
                    file_path: "main.ts".into(),
                    language: "typescript".into(),
                    imports: vec![],
                    file_namespace: None,
                };
                let model = SemanticModel::production();
                let mut actual = FxHashMap::default();
                for reference in file
                    .refs
                    .iter()
                    .filter(|r| r.kind == crate::types::EdgeKind::Calls)
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
                    let outcome = model.get_symbol_info(
                        &site,
                        &context,
                        &lookup,
                        &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
                    );
                    let info = match outcome {
                        SolveOutcome::Resolved(info) => info,
                        SolveOutcome::Unresolved(cause) => panic!(
                            "{} ({prefix}): generic cascade stopped at {selector}: {cause:?}",
                            case["name"]
                        ),
                        SolveOutcome::Drained => panic!(
                            "{} ({prefix}): generic cascade drained at {selector}",
                            case["name"]
                        ),
                    };
                    assert_eq!(
                        expected.get(&selector),
                        Some(&info.target_symbol_id),
                        "every extracted reference must match its exact compiler target"
                    );
                    actual.insert(selector, info.target_symbol_id);
                }
                assert_eq!(
                    actual, expected,
                    "{} ({prefix}): every source call must reach its labelled declaration",
                    case["name"]
                );
            };
            let mut config = context(&[&file]);
            config.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
                strict_parameters: true,
                strict_nulls: true,
                bivariant_methods: Some(true),
            });
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
fn method_heritage_matches_compiler_with_poisoned_portable_and_cold_evidence() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/method_heritage_fixtures.json"
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
            file.content = Some(source);
            reduce_to_contract(&mut file);
            let db = crate::Database::open_in_memory().unwrap();
            let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
                &db,
                std::slice::from_ref(&file),
                "internal",
                Some(&arena),
            )
            .unwrap();
            let catalog = owner(&ids, &file, "Catalog");
            for symbol in &mut file.symbols {
                symbol.name = "poisoned".into();
                symbol.qualified_name = "poisoned.display".into();
                symbol.signature = None;
            }
            let mut config = context(&[&file]);
            config.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
                strict_parameters: true,
                strict_nulls: true,
                bivariant_methods: Some(true),
            });
            let check = |tree: &Compilation| {
                let lookup = tree.program_lookup("main.ts").unwrap();
                assert_eq!(
                    lookup.symbol_by_id(catalog).is_some(),
                    case["admitted"].as_bool().unwrap(),
                    "{} ({prefix})",
                    case["name"]
                );
            };
            let tree = Compilation::build_with_context(
                &[file],
                &ids,
                Arc::clone(&arena),
                Some(&config),
                &HashSet::new(),
            );
            check(&tree);
            tree.persist_type_info(db.conn()).unwrap();
            let restored = Arc::new(TypeArena::new());
            restored.restore_snapshot(&arena.serialize_snapshot());
            let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
            cold.ingest_from_db(db.conn());
            check(&cold);
        }
    }
}
