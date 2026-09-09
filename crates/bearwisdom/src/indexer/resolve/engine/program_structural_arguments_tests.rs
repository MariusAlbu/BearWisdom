use super::super::super::merge_proof::tests::{context, owner, parse};
use super::*;
use crate::indexer::external_parse_payload::CachedParse;
use crate::indexer::programs::CallablePolicy;
use std::{collections::HashSet, sync::Arc};

#[test]
fn structural_constructor_inference_matches_compiler_poisoned_portable_and_cold() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/structural_initializer_fixtures.json"
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
            let field = owner(&ids, &file, "value");
            let expected = owner(&ids, &file, "expected");
            let mut targets = FxHashMap::default();
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
                targets.insert(selector as u32, ids.row_id("main.ts", slot).unwrap());
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
                let lookup = tree.program_lookup("main.ts").unwrap();
                let actual = lookup.field_type_id_of(field);
                if case["supported"].as_bool().unwrap() {
                    let expected = lookup.field_type_id_of(expected).unwrap();
                    assert_eq!(actual, Some(expected), "{} ({prefix})", case["name"]);
                } else {
                    assert!(
                        actual.is_none_or(|ty| matches!(
                            tree.type_arena().unwrap().get(ty),
                            Type::Unknown
                        )),
                        "{}: fabricated constructor",
                        case["name"]
                    );
                }
                if !targets.is_empty() {
                    exact_calls(tree, &file, &ids, &targets);
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

fn exact_calls(
    tree: &Compilation,
    file: &crate::types::ParsedFile,
    ids: &crate::indexer::symbol_ids::SymbolIds,
    expected: &FxHashMap<u32, i64>,
) {
    use crate::indexer::resolve::engine::{
        contract::{FileContext, FlowCacheLookup},
        file_lookup::FileLookup,
        semantic_model::{SemanticModel, SolveOutcome},
        testkit,
    };
    let lookup = FileLookup::for_file(tree, file, ids);
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
        let result = model.get_symbol_info(
            &site,
            &context,
            &lookup,
            &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
        );
        let info = match result {
            SolveOutcome::Resolved(info) => info,
            SolveOutcome::Unresolved(cause) => {
                panic!("constructor cascade stopped at {selector}: {cause:?}")
            }
            SolveOutcome::Drained => panic!("constructor cascade drained at {selector}"),
        };
        assert_eq!(
            expected.get(&selector),
            Some(&info.target_symbol_id),
            "each source call must reach its exact compiler target"
        );
        actual.insert(selector, info.target_symbol_id);
    }
    assert_eq!(&actual, expected);
}

#[test]
fn structural_constructor_provider_policy_edit_and_deletion_rebind_consumers() {
    use crate::indexer::symbol_ids::SymbolIds;
    let arena = Arc::new(TypeArena::new());
    let provider = "interface Payload { touch(): void } interface Source { read(): Payload } interface Target<T> { read(): T } interface Result<T> { read(): T } interface Factory { new<U>(input: Target<U>): Result<U> } declare const Build: Factory;";
    let files = parse(
        &arena,
        &[
            ("left.d.ts", provider),
            ("right.d.ts", provider),
            (
                "main.ts",
                "export {}; declare const input: Source; class Holder { value = new Build(input) }",
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
    let field = owner(&ids, &files[2], "value");
    let left = owner(&ids, &files[0], "Payload");
    let right = owner(&ids, &files[1], "Payload");
    let selected = |file: &crate::types::ParsedFile, policy: bool| {
        let mut config = context(&[file, &files[2]]);
        config.programs.as_mut().unwrap()[0].callable_policy = policy.then_some(CallablePolicy {
            strict_parameters: true,
            strict_nulls: true,
            bivariant_methods: Some(true),
        });
        config
    };
    let check = |tree: &Compilation, expected: Option<i64>| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        let arena = tree.type_arena().unwrap();
        let actual = lookup
            .field_type_id_of(field)
            .and_then(|ty| match arena.get(ty) {
                Type::Apply { args, .. } if args.len() == 1 => head_decl_id(arena, args[0]),
                _ => None,
            });
        assert_eq!(actual, expected);
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&selected(&files[0], true)),
        &HashSet::new(),
    );
    check(&tree, Some(left));
    tree.persist_type_info(db.conn()).unwrap();
    let old = tree
        .program_lookup("main.ts")
        .unwrap()
        .field_type_id_of(field)
        .unwrap();
    for (provider, policy, expected) in [
        (&files[1], true, Some(right)),
        (&files[1], false, None),
        (&files[1], true, Some(right)),
    ] {
        let mut changed = Compilation::build_with_context(
            &[],
            &SymbolIds::default(),
            Arc::clone(&arena),
            Some(&selected(provider, policy)),
            &HashSet::new(),
        );
        changed.ingest_from_db(db.conn());
        check(&changed, expected);
        assert!(
            !(&changed.program_lookup("main.ts").unwrap() as &dyn SymbolLookup)
                .accepts_type_context(&arena, old)
        );
        changed.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
        cold.ingest_from_db(db.conn());
        check(&cold, expected);
    }
    let invalid = parse(
        &arena,
        &[(
            "right.d.ts",
            &provider.replace(
                "interface Source { read(): Payload }",
                "interface Source { absent(): Payload }",
            ),
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
        Some(&selected(&invalid[0], true)),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, None);
    edited.persist_type_info(db.conn()).unwrap();
    assert!(edited
        .source_program_lookup(&files[1])
        .unwrap()
        .field_type_id_of(field)
        .is_none());
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, None);
    db.conn()
        .execute("DELETE FROM files WHERE path='right.d.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    deleted.ingest_from_db(db.conn());
    check(&deleted, None);
    let mut recovered = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&restored),
        Some(&selected(&files[0], true)),
        &HashSet::new(),
    );
    recovered.ingest_from_db(db.conn());
    check(&recovered, Some(left));
    recovered.persist_type_info(db.conn()).unwrap();
    let final_arena = Arc::new(TypeArena::new());
    final_arena.restore_snapshot(&restored.serialize_snapshot());
    let mut final_cold = Compilation::build(&[], &SymbolIds::default(), final_arena);
    final_cold.ingest_from_db(db.conn());
    check(&final_cold, Some(left));
}
