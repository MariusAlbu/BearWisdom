use super::super::merge_proof::tests::{context, owner, parse};
use super::*;
use std::{collections::HashSet, sync::Arc};

#[test]
fn generic_augmentation_cascades_keep_exact_targets_with_poisoned_portable_and_cold_inputs() {
    use crate::indexer::external_parse_payload::CachedParse;
    use crate::indexer::resolve::engine::{
        contract::{FileContext, FlowCacheLookup},
        file_lookup::FileLookup,
        semantic_model::{SemanticModel, SolveOutcome},
        testkit,
    };
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/generic_augmentation_fixtures.json"
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
                    assert!(
                        actual.insert(selector, info.target_symbol_id).is_none(),
                        "duplicate source call"
                    );
                }
                assert_eq!(
                    actual, expected,
                    "{} ({prefix}): every source call must reach its labelled declaration",
                    case["name"]
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
    }
}

#[test]
fn compiler_generic_augmentation_admission_preserves_negative_and_cold_evidence() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/generic_augmentation_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        for prefix in ["", "export {}; "] {
            use crate::indexer::{
                contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
            };
            let original = TypeArena::new();
            let arena = Arc::new(TypeArena::new());
            for n in 0..32 {
                arena.intern(Type::Literal(
                    crate::type_checker::core::types::LitValue::Int(n),
                ));
            }
            let source = format!("{prefix}{}", case["source"].as_str().unwrap());
            let parsed = parse(&original, &[("main.ts", &source)]).remove(0);
            let payload =
                serde_json::to_string(&CachedParse::from_parsed(&parsed, &original)).unwrap();
            let cached: CachedParse = serde_json::from_str(&payload).unwrap();
            let mut file =
                cached.into_parsed(&arena, "main.ts", &parsed.content_hash, parsed.size, None);
            file.content = Some(source);
            reduce_to_contract(&mut file);
            let mut files = vec![file];
            let db = crate::Database::open_in_memory().unwrap();
            let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
                &db,
                &files,
                "internal",
                Some(&arena),
            )
            .unwrap();
            let rows: Vec<_> = files[0]
                .symbols
                .iter()
                .enumerate()
                .filter(|(_, s)| s.name == "Catalog")
                .map(|(slot, _)| ids.row_id("main.ts", slot).unwrap())
                .collect();
            assert!(rows.len() >= 2);
            for symbol in &mut files[0].symbols {
                symbol.name = "poisoned".into();
                symbol.qualified_name = "poisoned.display".into();
                symbol.signature = None;
            }
            let check = |tree: &Compilation| {
                let lookup = tree.program_lookup("main.ts").unwrap();
                for &row in &rows {
                    assert_eq!(
                        lookup.symbol_by_id(row).is_some(),
                        case["admitted"].as_bool().unwrap(),
                        "{} ({prefix})",
                        case["name"]
                    );
                }
                if case["admitted"] == true {
                    let canonical = lookup.canonical_decl_id(rows[0]);
                    assert!(
                        rows.iter()
                            .all(|&row| lookup.canonical_decl_id(row) == canonical),
                        "{} ({prefix}): declaration group split",
                        case["name"]
                    );
                    let group = &lookup.view.generic_declarations[&canonical];
                    let info = lookup.canonical_type_info(canonical).unwrap();
                    assert_eq!(info.generic_param_ids, group.bound.generic_parameters);
                    assert_eq!(info.generic_param_default_ids, group.bound.defaults);
                    for (&parameter, &constraint) in group
                        .bound
                        .generic_parameters
                        .iter()
                        .zip(&group.bound.constraints)
                    {
                        assert_eq!(lookup.generic_constraint(parameter), Some(constraint));
                    }
                }
            };
            let tree = Compilation::build_with_context(
                &files,
                &ids,
                Arc::clone(&arena),
                Some(&context(&[&files[0]])),
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
fn generic_augmentation_preserves_raw_omissions_and_retargets_selected_defaults() {
    use crate::indexer::symbol_ids::SymbolIds;
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            ("base.d.ts", "interface Reader<T> { read(): T }"),
            (
                "left.d.ts",
                "interface Left { touch(): void } interface Reader<T extends Left = Left> {}",
            ),
            (
                "right.d.ts",
                "interface Right { touch(): void } interface Reader<T extends Right = Right> {}",
            ),
            ("main.ts", "export {}; declare const reader: Reader;"),
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
    let reader = owner(&ids, &files[0], "Reader");
    let left = owner(&ids, &files[1], "Left");
    let right = owner(&ids, &files[2], "Right");
    let check = |tree: &Compilation, expected: Option<i64>| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        assert_eq!(lookup.symbol_by_id(reader).is_some(), expected.is_some());
        if let Some(expected) = expected {
            let group = &lookup.view.generic_declarations[&lookup.canonical_decl_id(reader)];
            assert_eq!(group.bound.generic_parameters.len(), 1);
            let default = group.bound.defaults[0].unwrap();
            assert_eq!(
                super::super::super::head_decl::head_decl_id(tree.type_arena().unwrap(), default),
                Some(expected)
            );
            assert_eq!(group.bound.constraints[0], Some(default));
            let raw: Vec<_> = group
                .parts
                .iter()
                .map(|(_, bound, _)| bound.as_ref().unwrap())
                .collect();
            assert_eq!(
                raw.iter()
                    .filter(|b| b.defaults == [None] && b.constraints == [None])
                    .count(),
                1,
                "effective metadata must not rewrite omitted source declarations"
            );
            assert!(raw
                .iter()
                .all(|b| b.generic_parameters == group.bound.generic_parameters));
        }
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0], &files[1], &files[3]])),
        &HashSet::new(),
    );
    check(&tree, Some(left));
    tree.persist_type_info(db.conn()).unwrap();
    let mut switched = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&arena),
        Some(&context(&[&files[0], &files[2], &files[3]])),
        &HashSet::new(),
    );
    switched.ingest_from_db(db.conn());
    check(&switched, Some(right));
    switched.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, Some(right));
    // One selected augmentation becomes inconsistent with itself. The live,
    // unselected left namesake must not rescue its dependent default evidence.
    let changed = parse(
        &restored,
        &[(
            "right.d.ts",
            "interface Right { touch(): void } interface Reader<T extends Right = number> {}",
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
        Some(&context(&[&files[0], &changed[0], &files[3]])),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, None);
    edited.persist_type_info(db.conn()).unwrap();
    let edited_arena = Arc::new(TypeArena::new());
    edited_arena.restore_snapshot(&restored.serialize_snapshot());
    let mut edited_cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&edited_arena));
    edited_cold.ingest_from_db(db.conn());
    check(&edited_cold, None);
    db.conn()
        .execute("DELETE FROM files WHERE path='right.d.ts'", [])
        .unwrap();
    let mut stale = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&edited_arena));
    stale.ingest_from_db(db.conn());
    check(&stale, None);
    let mut deleted = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&edited_arena),
        Some(&context(&[&files[0], &files[1], &files[3]])),
        &HashSet::new(),
    );
    deleted.ingest_from_db(db.conn());
    check(&deleted, Some(left));
    deleted.persist_type_info(db.conn()).unwrap();
    let final_arena = Arc::new(TypeArena::new());
    final_arena.restore_snapshot(&edited_arena.serialize_snapshot());
    let mut final_cold = Compilation::build(&[], &SymbolIds::default(), final_arena);
    final_cold.ingest_from_db(db.conn());
    check(&final_cold, Some(left));
}
