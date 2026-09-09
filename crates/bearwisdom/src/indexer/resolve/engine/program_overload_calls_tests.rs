use super::super::merge_proof::tests::{context, owner, parse};
use super::*;
use crate::indexer::resolve::engine::{contract::FlowCacheLookup, file_lookup::FileLookup};
use std::{collections::HashSet, sync::Arc};

#[test]
fn inferred_predicate_uses_source_overload_order() {
    let source = "interface Factory<T> { pick<S extends T>(predicate: (value: T) => value is S): S | undefined; pick(predicate: (value: T) => unknown): T | undefined; } declare const api: Factory<string | number>; const result = api.pick(value => typeof value === 'string');";
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("main.ts", source)]);
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
    let lookup = FileLookup::for_file(&tree, &files[0], &ids);
    let selector = source.find("api.pick").unwrap() as u32 + 4;
    let args = lookup.source_call_arguments(selector).unwrap().unwrap();
    let actual =
        crate::indexer::resolve::engine::arg_types::resolve_arg_types(&lookup, &arena, args);
    let api = owner(&ids, &files[0], "api");
    let call = lookup
        .overloaded_call(
            lookup.field_type_id_of(api).unwrap(),
            selector,
            &actual,
            &[],
        )
        .unwrap()
        .expect("ordered predicate overload");
    assert_eq!(
        call.origins[call.selected].span.start,
        source.find("pick<S").unwrap() as u32
    );
}

#[test]
fn inline_callback_body_selects_ordinary_overload_without_shared_writes() {
    let source = "interface Factory<T> { pick<S extends T>(predicate: (value: T) => value is S): S | undefined; pick(predicate: (value: T) => unknown): T | undefined; } declare const api: Factory<string | number>; const result = api.pick(value => true);";
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("main.ts", source)]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let api = owner(&ids, &files[0], "api");
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
    let lookup = FileLookup::for_file(&tree, &files[0], &ids);
    let selector = source.find("api.pick").unwrap() as u32 + 4;
    let args = lookup.source_call_arguments(selector).unwrap().unwrap();
    let actual =
        crate::indexer::resolve::engine::arg_types::resolve_arg_types(&lookup, &arena, args);
    let call = lookup
        .overloaded_call(
            lookup.field_type_id_of(api).unwrap(),
            selector,
            &actual,
            &[],
        )
        .unwrap();
    let call = call.expect("source boolean body must disprove the predicate competitor");
    assert_eq!(
        call.origins[call.selected].span.start,
        source.find("pick(predicate").unwrap() as u32
    );
}

#[test]
fn configured_nested_callable_preserves_source_signature_evidence() {
    let arena = Arc::new(TypeArena::new());
    let source = "export {}; declare const callback: <T extends string>(value: T, flag?: boolean) => value is T;";
    let files = parse(&arena, &[("main.ts", source)]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let row = owner(&ids, &files[0], "callback");
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0]])),
        &HashSet::new(),
    );
    let lookup = tree.program_lookup("main.ts").unwrap();
    let ty = lookup.field_type_id_of(row).unwrap();
    let crate::type_checker::core::types::Type::Callable(c) = arena.get(ty) else {
        panic!(
            "configured callable lost source evidence: {:?}",
            arena.get(ty)
        );
    };
    assert!(c.complete);
    assert_eq!(c.origin.signature.start, source.find('<').unwrap() as u32);
    assert_eq!(
        c.predicate.as_ref().unwrap().parameter,
        c.parameters[0].declaration
    );
    assert!(c.parameters[1].optional);
    assert_eq!(c.generics[0].parameter, c.parameters[0].ty);
    assert_eq!(c.predicate.unwrap().asserted, Some(c.generics[0].parameter));
}

#[test]
fn compiler_callable_source_labels_survive_portable_and_cold_inputs() {
    use crate::indexer::external_parse_payload::CachedParse;
    use crate::type_checker::core::types::Type;
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/callable_identity_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        let source = case["source"].as_str().unwrap();
        let original = TypeArena::new();
        let parsed = parse(&original, &[("main.ts", source)]);
        let payload =
            serde_json::to_string(&CachedParse::from_parsed(&parsed[0], &original)).unwrap();
        let arena = Arc::new(TypeArena::new());
        for n in 0..32 {
            arena.intern(Type::Literal(
                crate::type_checker::core::types::LitValue::Int(n),
            ));
        }
        let cached: CachedParse = serde_json::from_str(&payload).unwrap();
        let mut file = cached.into_parsed(
            &arena,
            "main.ts",
            &parsed[0].content_hash,
            parsed[0].size,
            None,
        );
        file.content = Some(source.into());
        crate::indexer::contract_filter::reduce_to_contract(&mut file);
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            std::slice::from_ref(&file),
            "internal",
            Some(&arena),
        )
        .unwrap();
        let row = owner(&ids, &file, "callback");
        for symbol in &mut file.symbols {
            symbol.name = "poison".into();
            symbol.qualified_name = "poison.display".into();
            symbol.signature = None;
        }
        let check = |tree: &Compilation| {
            let lookup = tree.program_lookup("main.ts").unwrap();
            let arena = tree.type_arena().unwrap();
            let mut pending = vec![lookup
                .field_type_id_of(row)
                .unwrap_or_else(|| panic!("{}: missing callback field", case["name"]))];
            let mut seen = HashSet::new();
            let mut found = vec![];
            while let Some(ty) = pending.pop() {
                if !seen.insert(ty) {
                    continue;
                }
                if let Type::Callable(c) = arena.get(ty) {
                    pending.extend(c.operands().copied());
                    assert!(arena.accepts_nominal_context(ty, lookup.nominal_context()));
                    found.push(c);
                }
            }
            found.sort_by_key(|c| c.origin.signature.start);
            assert_eq!(
                found.len(),
                case["signatures"].as_array().unwrap().len(),
                "{}",
                case["name"]
            );
            for (c, label) in found.iter().zip(case["signatures"].as_array().unwrap()) {
                assert_eq!(
                    c.origin.signature.start,
                    source.find(label["start"].as_str().unwrap()).unwrap() as u32,
                    "{}",
                    case["name"]
                );
                assert_eq!(c.complete, !label["incomplete"].as_bool().unwrap_or(false));
                assert_eq!(
                    c.generics.len(),
                    label["generics"].as_u64().unwrap() as usize
                );
                assert_eq!(
                    c.parameters.len(),
                    label["parameters"].as_array().unwrap().len()
                );
                for (p, label) in c
                    .parameters
                    .iter()
                    .zip(label["parameters"].as_array().unwrap())
                {
                    assert_eq!(
                        p.declaration.start,
                        source.find(label["start"].as_str().unwrap()).unwrap() as u32
                    );
                    assert_eq!(
                        (p.optional, p.rest, p.receiver),
                        (
                            label["optional"].as_bool().unwrap(),
                            label["rest"].as_bool().unwrap(),
                            label["receiver"].as_bool().unwrap()
                        )
                    );
                }
                assert_eq!(c.predicate.is_some(), label.get("predicate").is_some());
                if let Some(p) = &c.predicate {
                    assert_eq!(
                        p.parameter.start,
                        source
                            .find(label["predicate"]["target"].as_str().unwrap())
                            .unwrap() as u32
                    );
                    assert_eq!(p.asserts, label["predicate"]["asserts"].as_bool().unwrap());
                    assert_eq!(
                        p.asserted.is_some(),
                        label["predicate"]["typed"].as_bool().unwrap()
                    );
                }
            }
            if found.len() == 2 {
                assert_eq!(found[0].parameters[0].ty, found[0].generics[0].parameter);
                if found[1].generics.is_empty() {
                    assert_eq!(found[1].parameters[0].ty, found[0].generics[0].parameter);
                } else {
                    assert_ne!(
                        found[1].generics[0].parameter,
                        found[0].generics[0].parameter
                    );
                    assert_eq!(found[1].parameters[0].ty, found[1].generics[0].parameter);
                    assert_eq!(found[1].result, found[1].generics[0].parameter);
                }
            }
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

#[test]
fn compiler_selected_overload_origins_and_results_fresh_and_cold() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/overload_call_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        let source = case["source"].as_str().unwrap();
        let arena = Arc::new(TypeArena::new());
        let files = parse(&arena, &[("main.ts", source)]);
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &files,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let api = owner(&ids, &files[0], "api");
        let selector = source.find("api.pick").unwrap() as u32 + 4;
        let expected = case["supported"]
            .as_bool()
            .unwrap()
            .then(|| owner(&ids, &files[0], "expected"));
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
        let check = |tree: &Compilation| {
            let lookup = FileLookup::for_file(tree, &files[0], &ids);
            let arena = tree.type_arena().unwrap();
            let receiver = lookup.field_type_id_of(api).expect("annotated receiver");
            let args = lookup.source_call_arguments(selector).unwrap().unwrap();
            let actual =
                crate::indexer::resolve::engine::arg_types::resolve_arg_types(&lookup, arena, args);
            let explicit = lookup.member_type_arguments(selector).unwrap_or(&[]);
            let selected = lookup.overloaded_call(receiver, selector, &actual, explicit);
            let selected = selected
                .unwrap_or_else(|| panic!("{}: source overload group missing", case["name"]));
            if let Some(expected) = expected {
                let selected =
                    selected.unwrap_or_else(|_| panic!("{}: no selected overload", case["name"]));
                assert_eq!(selected.origins.len(), 2, "retain navigation group");
                let origin = &selected.origins[selected.selected];
                assert_eq!(
                    origin.span.start,
                    source.find(case["selected"].as_str().unwrap()).unwrap() as u32,
                    "{}",
                    case["name"]
                );
                assert!(origin
                    .declaration
                    .and_then(|id| lookup.symbol_by_id(id))
                    .is_some());
                let program = tree.program_lookup("main.ts").unwrap();
                let relation = super::super::merge_proof::types::Relation {
                    lookup: &program,
                    arena,
                };
                let expected = relation
                    .canonical(lookup.field_type_id_of(expected).unwrap(), 0)
                    .unwrap();
                assert_eq!(selected.return_type, expected, "{}", case["name"]);
            } else {
                assert!(selected.is_err(), "{}: fabricated overload", case["name"]);
            }
        };
        check(&tree);
        tree.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &Default::default(), restored);
        cold.ingest_from_db(db.conn());
        check(&cold);
    }
}

#[test]
fn callable_query_origins_follow_provider_retarget_edit_and_deletion() {
    use crate::type_checker::core::types::{Intrinsic, Type};
    let provider = "export declare const callback: (value: string) => value is string;";
    let consumer =
        "import { callback } from './barrel'; export declare const captured: typeof callback;";
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            ("left.ts", provider),
            ("right.ts", provider),
            ("barrel.ts", "export { callback } from './left';"),
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
    let captured = owner(&ids, &files[3], "captured");
    let left = owner(&ids, &files[0], "callback");
    let right = owner(&ids, &files[1], "callback");
    let check = |tree: &Compilation, selected: Option<(i64, Intrinsic)>| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        let arena = tree.type_arena().unwrap();
        let ty = lookup.field_type_id_of(captured).unwrap();
        if let Some((row, expected)) = selected {
            assert_eq!(Some(ty), lookup.field_type_id_of(row));
            let Type::Callable(c) = arena.get(ty) else {
                panic!("source query must preserve callable origin");
            };
            assert_eq!(
                arena.get(c.predicate.unwrap().asserted.unwrap()),
                Type::Intrinsic(expected)
            );
            assert_ne!(
                lookup.field_type_id_of(left),
                lookup.field_type_id_of(right),
                "identical text is not source identity"
            );
        } else {
            assert!(matches!(arena.get(ty), Type::Unknown));
        }
        ty
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    let prior = check(&tree, Some((left, Intrinsic::String)));
    tree.persist_type_info(db.conn()).unwrap();
    let barrel = parse(
        &arena,
        &[("barrel.ts", "export { callback } from './right';")],
    );
    let (_, barrel_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &barrel,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut retargeted = Compilation::build_with_context(
        &barrel,
        &barrel_ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0], &files[1], &barrel[0], &files[3]])),
        &HashSet::new(),
    );
    retargeted.ingest_from_db(db.conn());
    check(&retargeted, Some((right, Intrinsic::String)));
    assert!(!arena.accepts_nominal_context(
        prior,
        retargeted
            .program_lookup("main.ts")
            .unwrap()
            .nominal_context()
    ));
    retargeted.persist_type_info(db.conn()).unwrap();
    let changed = parse(
        &arena,
        &[(
            "right.ts",
            "export declare const callback: (value: number) => value is number;",
        )],
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
        Some(&context(&[&files[0], &changed[0], &barrel[0], &files[3]])),
        &HashSet::new(),
    );
    let changed_right = owner(&changed_ids, &changed[0], "callback");
    edited.ingest_from_db(db.conn());
    check(&edited, Some((changed_right, Intrinsic::Number)));
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &Default::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, Some((changed_right, Intrinsic::Number)));
    db.conn()
        .execute("DELETE FROM files WHERE path='right.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build_with_context(
        &[],
        &Default::default(),
        restored,
        Some(&context(&[&files[0], &barrel[0], &files[3]])),
        &HashSet::new(),
    );
    deleted.ingest_from_db(db.conn());
    check(&deleted, None);
}

#[test]
fn overloaded_result_cascades_ignore_poisoned_display_operands() {
    use crate::indexer::resolve::engine::{
        contract::FileContext,
        semantic_model::{SemanticModel, SolveOutcome},
        testkit,
    };
    use crate::types::{CallArg, EdgeKind};
    let source = "export {}; interface TextResult { text(): void } interface NumberResult { number(): void } interface Factory { pick(value: string): TextResult; pick(value: number): NumberResult; } function run(api: Factory) { const result = api.pick(42); result.number(); api.pick(42).number(); }";
    let arena = Arc::new(TypeArena::new());
    let mut files = parse(&arena, &[("main.ts", source)]);
    for reference in &mut files[0].refs {
        reference.call_args = vec![CallArg::Ident("poisoned".into())];
        for segment in reference.chain.iter_mut().flat_map(|c| &mut c.segments) {
            segment.name = "poisoned".into();
            segment.call_args.clear();
        }
    }
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let target = owner(&ids, &files[0], "number");
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0]])),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let lookup = FileLookup::for_file(tree, &files[0], &ids);
        let file = FileContext {
            file_path: "main.ts".into(),
            language: "typescript".into(),
            imports: vec![],
            file_namespace: None,
        };
        let model = SemanticModel::production();
        let mut downstream = std::collections::HashSet::new();
        for (index, reference) in files[0]
            .refs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == EdgeKind::Calls)
        {
            let mut context = testkit::ref_ctx(
                reference,
                &files[0].symbols[reference.source_symbol_index],
                vec![],
            );
            context.source_symbol_id = ids.row_id("main.ts", reference.source_symbol_index);
            lookup.set_cursor(reference.byte_offset);
            let SolveOutcome::Resolved(info) = model.get_symbol_info(
                &context,
                &file,
                &lookup,
                &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
            ) else {
                panic!("call at {} did not resolve", reference.byte_offset);
            };
            if let Some(ty) = info.resolved_yield_type {
                lookup.record_rhs_type(index, "poisoned", ty);
            }
            if info.target_symbol_id == target {
                downstream.insert(
                    reference
                        .chain
                        .as_ref()
                        .unwrap()
                        .segments
                        .last()
                        .unwrap()
                        .byte_offset,
                );
            }
        }
        assert_eq!(
            downstream.len(),
            2,
            "both local and inline result cascades must flip"
        );
    };
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &Default::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
}

#[test]
fn callable_argument_selection_flips_local_and_inline_return_cascades() {
    use crate::indexer::resolve::engine::{
        contract::FileContext,
        semantic_model::{SemanticModel, SolveOutcome},
        testkit,
    };
    use crate::types::{CallArg, EdgeKind};
    let source = "export {}; interface TextResult { text(): void } interface NumberResult { number(): void } interface Factory { pick(callback: () => string): TextResult; pick(callback: () => number): NumberResult; } declare const callback: () => number; function run(api: Factory) { const result = api.pick(callback); result.number(); api.pick(callback).number(); }";
    for argument in ["callback", "() => 42", "() => { return 42; }"] {
        for competing in [false, true] {
            let source = if competing {
                source
                    .replace("() => string", "() => number")
                    .replace("result.number()", "result.text()")
                    .replace(").number()", ").text()")
            } else {
                source.to_string()
            };
            for nullable in [false, true] {
                let source = source.replace("pick(callback)", &format!("pick({argument})"));
                let source = if nullable {
                    source
                        .replace("): NumberResult;", "): NumberResult | undefined;")
                        .replace("): TextResult;", "): TextResult | undefined;")
                        .replace(".number()", "?.number()")
                        .replace(".text()", "?.text()")
                } else {
                    source
                };
                let arena = Arc::new(TypeArena::new());
                let mut files = parse(&arena, &[("main.ts", &source)]);
                for reference in &mut files[0].refs {
                    reference.call_args = vec![CallArg::Ident("poisoned".into())];
                    for segment in reference.chain.iter_mut().flat_map(|c| &mut c.segments) {
                        segment.name = "poisoned".into();
                        segment.call_args.clear();
                    }
                }
                let db = crate::Database::open_in_memory().unwrap();
                let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
                    &db,
                    &files,
                    "internal",
                    Some(&arena),
                )
                .unwrap();
                let target = owner(&ids, &files[0], if competing { "text" } else { "number" });
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
                let check = |tree: &Compilation| {
                    let lookup = FileLookup::for_file(tree, &files[0], &ids);
                    let file = FileContext {
                        file_path: "main.ts".into(),
                        language: "typescript".into(),
                        imports: vec![],
                        file_namespace: None,
                    };
                    let model = SemanticModel::production();
                    let mut downstream = HashSet::new();
                    for (index, reference) in files[0]
                        .refs
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| r.kind == EdgeKind::Calls)
                    {
                        let mut context = testkit::ref_ctx(
                            reference,
                            &files[0].symbols[reference.source_symbol_index],
                            vec![],
                        );
                        context.source_symbol_id =
                            ids.row_id("main.ts", reference.source_symbol_index);
                        lookup.set_cursor(reference.byte_offset);
                        let SolveOutcome::Resolved(info) = model.get_symbol_info(
                            &context,
                            &file,
                            &lookup,
                            &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
                        ) else {
                            panic!("call at {} did not resolve", reference.byte_offset);
                        };
                        if let Some(ty) = info.resolved_yield_type {
                            lookup.record_rhs_type(index, "poisoned", ty);
                        }
                        if info.target_symbol_id == target {
                            downstream.insert(
                                reference
                                    .chain
                                    .as_ref()
                                    .unwrap()
                                    .segments
                                    .last()
                                    .unwrap()
                                    .byte_offset,
                            );
                        }
                    }
                    assert_eq!(downstream.len(), 2);
                };
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
}

#[test]
fn overload_provider_retarget_edit_and_deletion_preserve_source_ownership() {
    let provider =
        "export interface Factory { pick(value: string): string; pick(value: number): number; }";
    let consumer = "import { Factory } from './barrel'; declare const api: Factory; const result = api.pick(42);";
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            ("left.ts", provider),
            ("right.ts", provider),
            ("barrel.ts", "export { Factory } from './left';"),
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
    let api = owner(&ids, &files[3], "api");
    let selector = consumer.find("api.pick").unwrap() as u32 + 4;
    let check = |tree: &Compilation, path: Option<&str>| {
        let lookup = FileLookup::for_file(tree, &files[3], &ids);
        let arena = tree.type_arena().unwrap();
        let arguments = lookup.source_call_arguments(selector).unwrap().unwrap();
        let actual = crate::indexer::resolve::engine::arg_types::resolve_arg_types(
            &lookup, arena, arguments,
        );
        let call = lookup
            .field_type_id_of(api)
            .and_then(|receiver| lookup.overloaded_call(receiver, selector, &actual, &[]))
            .and_then(Result::ok);
        if let Some(path) = path {
            let call = call.expect("unique overload remains applicable");
            assert_eq!(call.origins.len(), 2);
            for origin in &call.origins {
                assert_eq!(
                    lookup
                        .symbol_by_id(origin.declaration.unwrap())
                        .unwrap()
                        .file_path
                        .as_ref(),
                    path
                );
            }
            assert_eq!(call.origins[0].source, call.origins[1].source);
        } else {
            assert!(
                call.is_none(),
                "missing/inapplicable provider cannot borrow a namesake"
            );
        }
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    check(&tree, Some("left.ts"));
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(
        &arena,
        &[("barrel.ts", "export { Factory } from './right';")],
    );
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let selected = context(&[&files[0], &files[1], &changed[0], &files[3]]);
    let mut retargeted = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&selected),
        &HashSet::new(),
    );
    retargeted.ingest_from_db(db.conn());
    check(&retargeted, Some("right.ts"));
    retargeted.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &Default::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, Some("right.ts"));
    let invalid = parse(&restored, &[("right.ts", "export interface Factory { pick(value: string): string; pick(value: boolean): boolean; }")]);
    let (_, invalid_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &invalid,
        "internal",
        Some(&restored),
    )
    .unwrap();
    let selected = context(&[&files[0], &invalid[0], &changed[0], &files[3]]);
    let mut edited = Compilation::build_with_context(
        &invalid,
        &invalid_ids,
        Arc::clone(&restored),
        Some(&selected),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, None);
    edited.persist_type_info(db.conn()).unwrap();
    db.conn()
        .execute("DELETE FROM files WHERE path='right.ts'", [])
        .unwrap();
    let remaining = context(&[&files[0], &changed[0], &files[3]]);
    let mut deleted = Compilation::build_with_context(
        &[],
        &Default::default(),
        restored,
        Some(&remaining),
        &HashSet::new(),
    );
    deleted.ingest_from_db(db.conn());
    check(&deleted, None);
}

#[test]
fn rowless_overloads_keep_portable_signatures_and_source_origins() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let original = TypeArena::new();
    let source = "function discard() {} interface Factory { pick(value: string): string; pick(value: number): number; } declare const api: Factory; const result = api.pick(42);";
    let mut files = parse(&original, &[("main.ts", source)]);
    let discard = files[0]
        .symbols
        .iter()
        .position(|s| s.name == "discard")
        .unwrap();
    for symbol in files[0].symbols.iter_mut().filter(|s| s.name == "pick") {
        symbol.parent_index = Some(discard);
    }
    reduce_to_contract(&mut files[0]);
    assert!(!files[0].symbols.iter().any(|s| s.name == "pick"));
    let payload = serde_json::to_string(&CachedParse::from_parsed(&files[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    for value in 0..40 {
        arena.intern(crate::type_checker::core::types::Type::Literal(
            crate::type_checker::core::types::LitValue::Int(value),
        ));
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
    let api = owner(&ids, &file, "api");
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        let arena = tree.type_arena().unwrap();
        let name = lookup.member_index().unwrap().name("pick").unwrap();
        let receiver = lookup.field_type_id_of(api).unwrap();
        let actual = arena.intern(crate::type_checker::core::types::Type::Literal(
            crate::type_checker::core::types::LitValue::Int(42),
        ));
        let call = select(&lookup, receiver, name, &[actual], &[])
            .unwrap()
            .unwrap();
        assert_eq!(call.origins.len(), 2);
        assert!(call
            .origins
            .iter()
            .all(|origin| origin.declaration.is_none()));
        assert_eq!(
            call.origins[call.selected].span.start,
            source.find("pick(value: number)").unwrap() as u32
        );
        assert_eq!(
            arena.get(call.return_type),
            crate::type_checker::core::types::Type::Intrinsic(
                crate::type_checker::core::types::Intrinsic::Number
            )
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
    for symbol in &mut file.symbols {
        symbol.name = "poison".into();
        symbol.qualified_name = "poison.display".into();
        symbol.signature = None;
    }
    let poisoned = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&file])),
        &HashSet::new(),
    );
    check(&poisoned);
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &Default::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
}
