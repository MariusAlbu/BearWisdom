use super::super::super::merge_proof::tests::{context, owner, parse};
use super::*;
use crate::indexer::resolve::engine::{
    contract::{FileContext, FlowCacheLookup},
    file_lookup::FileLookup,
    semantic_model::{SemanticModel, SolveOutcome},
    testkit,
};
use crate::indexer::{external_parse_payload::CachedParse, programs::CallablePolicy};
use std::{collections::HashSet, sync::Arc};
#[path = "program_object_state_tests.rs"]
mod state;

#[test]
fn object_initializer_compiler_cases_survive_portable_poisoned_and_cold_inputs() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/object_initializer_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        let source = case["source"].as_str().unwrap();
        let original = TypeArena::new();
        let parsed = parse(&original, &[("main.ts", source)]).remove(0);
        let payload = serde_json::to_string(&CachedParse::from_parsed(&parsed, &original)).unwrap();
        let arena = Arc::new(TypeArena::new());
        for n in 0..32 {
            arena.intern(Type::Literal(
                crate::type_checker::core::types::LitValue::Int(n),
            ));
        }
        let cached: CachedParse = serde_json::from_str(&payload).unwrap();
        let mut file =
            cached.into_parsed(&arena, "main.ts", &parsed.content_hash, parsed.size, None);
        file.content = Some(source.into());
        crate::indexer::contract_bindings::restore(&mut file);
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            std::slice::from_ref(&file),
            "internal",
            Some(&arena),
        )
        .unwrap();
        let labels: Vec<_> = case["calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|label| {
                let expression = label["expression"].as_str().unwrap();
                let selector = source.find(expression).unwrap()
                    + expression[..expression.find('(').unwrap()]
                        .rfind('.')
                        .unwrap()
                    + 1;
                let declaration = source.find(label["declaration"].as_str().unwrap()).unwrap();
                let row = file
                    .symbols
                    .iter()
                    .position(|s| s.start_line == 0 && s.start_col as usize == declaration)
                    .and_then(|slot| ids.row_id("main.ts", slot));
                (label, selector as u32, row)
            })
            .collect();
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
            let arena = tree.type_arena().unwrap();
            let lookup = FileLookup::for_file(tree, &file, &ids);
            let ctx = FileContext {
                file_path: "main.ts".into(),
                language: "typescript".into(),
                imports: vec![],
                file_namespace: None,
            };
            lookup.set_cursor((source.len() - 1) as u32);
            for label in case["reads"].as_array().into_iter().flatten() {
                assert_eq!(
                    lookup
                        .local_type_id(label["name"].as_str().unwrap())
                        .map(|ty| arena.format_type(ty)),
                    Some(label["type"].as_str().unwrap().into()),
                    "{}: {}",
                    case["name"],
                    label["name"]
                );
            }
            for (label, selector, expected) in &labels {
                let reference = file
                    .refs
                    .iter()
                    .find(|r| {
                        r.kind == crate::types::EdgeKind::Calls
                            && r.chain.as_ref().is_some_and(|c| {
                                c.segments.last().unwrap().byte_offset == *selector
                            })
                    })
                    .unwrap();
                let mut site = testkit::ref_ctx(
                    reference,
                    &file.symbols[reference.source_symbol_index],
                    vec![],
                );
                site.source_symbol_id = ids.row_id("main.ts", reference.source_symbol_index);
                lookup.set_cursor(reference.byte_offset);
                let result = SemanticModel::production().get_symbol_info(
                    &site,
                    &ctx,
                    &lookup,
                    &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
                );
                if !label["supported"].as_bool().unwrap() {
                    assert!(
                        !matches!(result, SolveOutcome::Resolved(_)),
                        "{}: incomplete source must not reach a namesake",
                        case["name"]
                    );
                    continue;
                }
                if case["nominal"] != true {
                    let chain = reference.chain.as_ref().unwrap();
                    let mut receiver = lookup.local_type_id("api").expect("source object root");
                    for segment in chain.segments.iter().skip(1).take(chain.segments.len() - 2) {
                        receiver = lookup
                            .source_object_member(receiver, segment.byte_offset)
                            .unwrap()
                            .ok()
                            .unwrap()
                            .value;
                    }
                    let arguments = lookup.source_call_arguments(*selector).unwrap().unwrap();
                    let actual = crate::indexer::resolve::engine::arg_types::resolve_arg_types(
                        &lookup, arena, arguments,
                    );
                    let selected = lookup
                        .overloaded_call(receiver, *selector, &actual, &[])
                        .unwrap()
                        .ok()
                        .unwrap();
                    let origin = &selected.origins[selected.selected];
                    assert_eq!(
                        origin.span.start,
                        source.find(label["signature"].as_str().unwrap()).unwrap() as u32,
                        "{}: source signature",
                        case["name"]
                    );
                    assert_eq!(
                        origin.declaration, *expected,
                        "{}: navigation origin",
                        case["name"]
                    );
                    assert_eq!(
                        arena.format_type(selected.return_type),
                        label["result"].as_str().unwrap(),
                        "{}: call type",
                        case["name"]
                    );
                }
                if expected.is_none() {
                    assert!(
                        !matches!(result, SolveOutcome::Resolved(_)),
                        "{}: missing row cannot fabricate navigation",
                        case["name"]
                    );
                    continue;
                }
                let SolveOutcome::Resolved(info) = result else {
                    panic!(
                        "{}: source object member at {selector}, expected {expected:?}",
                        case["name"]
                    );
                };
                assert_eq!(Some(info.target_symbol_id), *expected, "{}", case["name"]);
                assert_eq!(
                    info.resolved_yield_type.map(|ty| arena.format_type(ty)),
                    Some(label["result"].as_str().unwrap().into()),
                    "{}",
                    case["name"]
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

#[test]
fn iife_return_object_reaches_exact_imported_method_declarations() {
    let arena = Arc::new(TypeArena::new());
    let provider = "export const api = (() => { let state: () => boolean = () => true; return { check(): boolean { return state() }, replace(next: () => boolean): void { state = next } }; })();";
    let consumer =
        "import { api } from './provider'; const result = api.check(); api.replace(() => false);";
    let files = parse(&arena, &[("provider.ts", provider), ("main.ts", consumer)]);
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
        Some(&context(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    let expected = owner(&ids, &files[0], "check");
    let lookup = FileLookup::for_file(&tree, &files[1], &ids);
    let reference = files[1]
        .refs
        .iter()
        .find(|r| {
            r.kind == crate::types::EdgeKind::Calls
                && r.chain
                    .as_ref()
                    .is_some_and(|c| c.segments.last().unwrap().name == "check")
        })
        .unwrap();
    let mut site = testkit::ref_ctx(
        reference,
        &files[1].symbols[reference.source_symbol_index],
        vec![],
    );
    site.source_symbol_id = ids.row_id("main.ts", reference.source_symbol_index);
    lookup.set_cursor(reference.byte_offset);
    let ctx = FileContext {
        file_path: "main.ts".into(),
        language: "typescript".into(),
        imports: vec![],
        file_namespace: None,
    };
    let result = SemanticModel::production().get_symbol_info(
        &site,
        &ctx,
        &lookup,
        &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
    );
    let SolveOutcome::Resolved(info) = result else {
        eprintln!(
            "provider initializers {:?}",
            files[0].flow.lexical.as_ref().unwrap().types.initializers
        );
        panic!("source-owned IIFE object must reach the imported method");
    };
    assert_eq!(info.target_symbol_id, expected);
    assert_eq!(
        info.resolved_yield_type,
        Some(arena.intern(Type::Intrinsic(
            crate::type_checker::core::types::Intrinsic::Boolean
        )))
    );
}
