use super::merge_proof::tests::{context, owner, parse};
use super::merge_proof::types::Relation;
use super::*;
use crate::indexer::{programs::CompilerIntrinsicPolicy, symbol_ids::SymbolIds};
use crate::type_checker::core::types::{Intrinsic, Type};
use std::{collections::HashSet, sync::Arc};

fn canonical(tree: &Compilation, path: &str, row: i64) -> Option<Type> {
    let lookup = tree.program_lookup(path)?;
    let arena = tree.type_arena()?;
    let ty = (&lookup as &dyn SymbolLookup).declaration_type(arena, row)?;
    Relation {
        lookup: &lookup,
        arena,
    }
    .canonical(ty, 0)
    .map(|ty| arena.get(ty))
}

#[test]
fn compiler_intrinsic_fixtures_preserve_portable_cold_and_namesake_evidence() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/compiler_intrinsic_fixtures.json"
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
            reduce_to_contract(&mut file);
            let db = crate::Database::open_in_memory().unwrap();
            let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
                &db,
                std::slice::from_ref(&file),
                "internal",
                Some(&arena),
            )
            .unwrap();
            let row = |declaration: &str| {
                let byte = source.find(declaration).unwrap();
                let slot = file
                    .symbols
                    .iter()
                    .position(|s| s.start_line == 0 && s.start_col as usize == byte)
                    .unwrap();
                ids.row_id("main.ts", slot).unwrap()
            };
            let mut expected = Vec::new();
            for label in case["types"].as_array().into_iter().flatten() {
                let declaration = label[0].as_str().unwrap();
                if case["unsupported"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|v| v == declaration))
                {
                    continue;
                }
                let kind = match label[1].as_str().unwrap() {
                    "undefined" => Intrinsic::Undefined,
                    "any" => Intrinsic::Any,
                    "string" => Intrinsic::String,
                    "number" => Intrinsic::Number,
                    _ => panic!("unhandled gold {label}"),
                };
                expected.push((row(declaration), Some(Type::Intrinsic(kind))));
            }
            for field in ["unknown", "unsupported", "rejected"] {
                for declaration in case[field].as_array().into_iter().flatten() {
                    expected.push((row(declaration.as_str().unwrap()), None));
                }
            }
            for symbol in &mut file.symbols {
                symbol.name = "poisoned".into();
                symbol.qualified_name = "poisoned.display".into();
                symbol.signature = None;
            }
            let mut config = context(&[&file]);
            config.programs.as_mut().unwrap()[0].compiler_intrinsics =
                crate::resolution_oracle::compiler_intrinsic_policy::typescript_options(
                    &case["options"],
                );
            let check = |tree: &Compilation| {
                for (row, expected) in &expected {
                    assert_eq!(
                        canonical(tree, "main.ts", *row),
                        *expected,
                        "{} ({prefix}): {row}",
                        case["name"]
                    );
                }
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
            let mut cold = Compilation::build(&[], &Default::default(), restored);
            cold.ingest_from_db(db.conn());
            check(&cold);
        }
    }
}

#[test]
fn compiler_intrinsic_policy_provider_edits_and_deletion_rebuild_from_source_evidence() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            ("builtin.d.ts", "type BuiltinIteratorReturn = intrinsic;"),
            ("ordinary.d.ts", "type BuiltinIteratorReturn = string;"),
            ("main.ts", "export {}; type Result = BuiltinIteratorReturn;"),
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
    let builtin = owner(&ids, &files[0], "BuiltinIteratorReturn");
    let result = owner(&ids, &files[2], "Result");
    let make_config = |provider: &crate::types::ParsedFile, policy: Option<bool>| {
        let mut config = context(&[provider, &files[2]]);
        config.programs.as_mut().unwrap()[0].compiler_intrinsics =
            policy.map(|strict_iterator_return| CompilerIntrinsicPolicy {
                strict_iterator_return,
            });
        config
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&make_config(&files[0], Some(true))),
        &HashSet::new(),
    );
    assert_eq!(
        canonical(&tree, "main.ts", result),
        Some(Type::Intrinsic(Intrinsic::Undefined))
    );
    let old_lookup = tree.program_lookup("main.ts").unwrap();
    let old_type = (&old_lookup as &dyn SymbolLookup)
        .declaration_type(&arena, builtin)
        .unwrap();
    tree.persist_type_info(db.conn()).unwrap();
    for (provider, policy, expected) in [
        (&files[0], Some(false), Some(Intrinsic::Any)),
        (&files[0], None, None),
        (&files[1], None, Some(Intrinsic::String)),
        (&files[0], Some(true), Some(Intrinsic::Undefined)),
    ] {
        let mut switched = Compilation::build_with_context(
            &[],
            &SymbolIds::default(),
            Arc::clone(&arena),
            Some(&make_config(provider, policy)),
            &HashSet::new(),
        );
        switched.ingest_from_db(db.conn());
        assert_eq!(
            canonical(&switched, "main.ts", result),
            expected.map(Type::Intrinsic)
        );
        let lookup = switched.program_lookup("main.ts").unwrap();
        assert!(!(&lookup as &dyn SymbolLookup).accepts_type_context(&arena, old_type));
        switched.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
        cold.ingest_from_db(db.conn());
        assert_eq!(
            canonical(&cold, "main.ts", result),
            expected.map(Type::Intrinsic)
        );
    }
    let changed = parse(
        &arena,
        &[("builtin.d.ts", "type BuiltinIteratorReturn = number;")],
    );
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut edited = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&make_config(&changed[0], Some(true))),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    assert_eq!(
        canonical(&edited, "main.ts", result),
        Some(Type::Intrinsic(Intrinsic::Number))
    );
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    assert_eq!(
        canonical(&cold, "main.ts", result),
        Some(Type::Intrinsic(Intrinsic::Number))
    );
    assert!(cold
        .source_program_lookup(&files[0])
        .unwrap()
        .symbol_by_id(builtin)
        .is_none());
    db.conn()
        .execute("DELETE FROM files WHERE path='builtin.d.ts'", [])
        .unwrap();
    let mut stale = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    stale.ingest_from_db(db.conn());
    assert_eq!(canonical(&stale, "main.ts", result), None);
    let mut deleted = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&restored),
        Some(&make_config(&files[1], Some(true))),
        &HashSet::new(),
    );
    deleted.ingest_from_db(db.conn());
    assert_eq!(
        canonical(&deleted, "main.ts", result),
        Some(Type::Intrinsic(Intrinsic::String))
    );
}

#[test]
fn compiler_intrinsic_alias_has_a_source_owned_body() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[("main.ts", "type BuiltinIteratorReturn = intrinsic;")],
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
    config.programs.as_mut().unwrap()[0].compiler_intrinsics =
        Some(crate::indexer::programs::CompilerIntrinsicPolicy {
            strict_iterator_return: true,
        });
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    let lookup = tree.program_lookup("main.ts").unwrap();
    let body = lookup
        .canonical_type_info(owner(&ids, &files[0], "BuiltinIteratorReturn"))
        .unwrap()
        .lexical_alias
        .as_ref()
        .unwrap()
        .instantiate(&arena, &[])
        .unwrap();
    assert_eq!(arena.get(body), Type::Intrinsic(Intrinsic::Undefined));
}

#[test]
fn compiler_intrinsic_return_cascades_keep_exact_source_calls_and_unknown_barriers() {
    use crate::indexer::external_parse_payload::CachedParse;
    use crate::indexer::resolve::engine::{
        contract::{FileContext, FlowCacheLookup},
        file_lookup::FileLookup,
        semantic_model::{SemanticModel, SolveOutcome},
        testkit,
    };
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/compiler_intrinsic_fixtures.json"
    ))
    .unwrap();
    for case in cases.iter().filter(|c| c["calls"].is_array()) {
        for prefix in ["", "export {}; "] {
            let source = format!("{prefix}{}", case["source"].as_str().unwrap());
            let original = TypeArena::new();
            let parsed = parse(&original, &[("main.ts", &source)]).remove(0);
            let payload =
                serde_json::to_string(&CachedParse::from_parsed(&parsed, &original)).unwrap();
            let arena = Arc::new(TypeArena::new());
            arena.class("shift source type identities");
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
            let check = |tree: &Compilation, supported: bool| {
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
                    match outcome {
                        SolveOutcome::Resolved(info) => {
                            assert!(supported);
                            actual.insert(selector, info.target_symbol_id);
                        }
                        _ => assert!(
                            !supported,
                            "{} ({prefix}): cascade stopped at {selector}",
                            case["name"]
                        ),
                    }
                }
                assert_eq!(
                    actual,
                    if supported {
                        expected.clone()
                    } else {
                        FxHashMap::default()
                    }
                );
            };
            let mut config = context(&[&file]);
            config.programs.as_mut().unwrap()[0].compiler_intrinsics =
                crate::resolution_oracle::compiler_intrinsic_policy::typescript_options(
                    &case["options"],
                );
            let tree = Compilation::build_with_context(
                std::slice::from_ref(&file),
                &ids,
                Arc::clone(&arena),
                Some(&config),
                &HashSet::new(),
            );
            let supported = case["cascadeUnsupported"].is_null();
            check(&tree, supported);
            tree.persist_type_info(db.conn()).unwrap();
            let restored = Arc::new(TypeArena::new());
            restored.restore_snapshot(&arena.serialize_snapshot());
            let mut cold = Compilation::build(&[], &Default::default(), Arc::clone(&restored));
            cold.ingest_from_db(db.conn());
            check(&cold, supported);
            config.programs.as_mut().unwrap()[0].compiler_intrinsics = None;
            let mut missing = Compilation::build_with_context(
                &[],
                &Default::default(),
                restored,
                Some(&config),
                &HashSet::new(),
            );
            missing.ingest_from_db(db.conn());
            check(&missing, false);
        }
    }
}
