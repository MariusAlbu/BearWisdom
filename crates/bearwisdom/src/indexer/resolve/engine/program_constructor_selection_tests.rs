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

#[test]
fn compiler_constructor_selection_survives_portable_poisoned_cold_views() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/constructor_selection_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        for rowless in [false, true] {
            let sources: Vec<_> = case["sources"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(path, text)| (path.as_str(), text.as_str().unwrap()))
                .collect();
            let original = TypeArena::new();
            let parsed = parse(&original, &sources);
            let arena = Arc::new(TypeArena::new());
            for n in 0..32 {
                arena.intern(Type::Literal(
                    crate::type_checker::core::types::LitValue::Int(n),
                ));
            }
            let mut files: Vec<_> = parsed
                .iter()
                .map(|input| {
                    let payload =
                        serde_json::to_string(&CachedParse::from_parsed(input, &original)).unwrap();
                    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
                    let mut file = cached.into_parsed(
                        &arena,
                        &input.path,
                        &input.content_hash,
                        input.size,
                        None,
                    );
                    file.content = input.content.clone();
                    crate::indexer::contract_bindings::restore(&mut file);
                    file
                })
                .collect();
            if rowless {
                for file in &mut files {
                    let graph = file.flow.lexical.as_mut().unwrap();
                    for part in &mut graph.globals.as_mut().unwrap().interfaces {
                        for member in part
                            .surface
                            .iter_mut()
                            .flatten()
                            .filter(|member| member.kind == Kind::Construct)
                        {
                            member.slot = None;
                        }
                    }
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
            let main = files.iter().position(|f| f.path == "main.ts").unwrap();
            let expected = case["supported"]
                .as_bool()
                .unwrap()
                .then(|| owner(&ids, &files[main], "expected"));
            let mut targets = FxHashMap::default();
            for reference in files[main]
                .refs
                .iter()
                .filter(|r| r.kind == crate::types::EdgeKind::Calls)
            {
                let terminal = reference.chain.as_ref().unwrap().segments.last().unwrap();
                let mut declarations = files.iter().flat_map(|file| {
                    file.symbols
                        .iter()
                        .enumerate()
                        .filter(|(_, symbol)| {
                            symbol.name == terminal.name
                                && matches!(symbol.kind, crate::types::SymbolKind::Method)
                        })
                        .map(|(slot, _)| ids.row_id(&file.path, slot).unwrap())
                });
                if let Some(row) = declarations.next() {
                    assert!(declarations.next().is_none());
                    targets.insert(terminal.byte_offset, row);
                }
            }
            for file in &mut files {
                for symbol in &mut file.symbols {
                    symbol.name = "poison".into();
                    symbol.qualified_name = "poison.display".into();
                    symbol.signature = None;
                }
            }
            for reference in &mut files[main].refs {
                reference.call_args = vec![crate::types::CallArg::Ident("poison".into())];
                for segment in reference.chain.iter_mut().flat_map(|c| &mut c.segments) {
                    segment.name = "poison".into();
                    segment.call_args.clear();
                }
            }
            let mut config = context(&files.iter().collect::<Vec<_>>());
            config.programs.as_mut().unwrap()[0].source_binding_order = Some(
                case["roots"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().into())
                    .collect(),
            );
            config.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
                strict_parameters: true,
                strict_nulls: true,
                bivariant_methods: Some(true),
            });
            let check = |tree: &Compilation| {
                let lookup = tree.program_lookup("main.ts").unwrap();
                let start = case["sources"]["main.ts"]
                    .as_str()
                    .unwrap()
                    .find("actual =")
                    .unwrap() as u32;
                let source = lookup.source.unwrap();
                let (&signature, &actual) = source
                    .initializers
                    .iter()
                    .find(|(id, _)| id.0.start == start)
                    .unwrap();
                let wanted = expected.map(|row| lookup.field_type_id_of(row).unwrap());
                assert_eq!(
                    actual, wanted,
                    "{}: selected constructor result at {:?}",
                    case["name"], signature
                );
                let call = source
                    .constructor_calls
                    .get(&signature)
                    .and_then(Option::as_ref);
                if wanted.is_none() {
                    assert!(
                        call.is_none(),
                        "{}: no provisional constructor origin",
                        case["name"]
                    );
                    return;
                }
                let call = call.expect("proved source constructor evidence");
                assert_eq!(Some(call.return_type), wanted);
                let selected = call.selected.expect("exact selected constructor index");
                let origin = &call.origins[selected];
                if case["selected"].is_null() {
                    assert!(
                        origin.is_none(),
                        "implicit constructor has no source signature"
                    );
                } else {
                    let origin = origin.as_ref().expect("source signature origin");
                    let provider = case["selectedFile"].as_str().unwrap();
                    assert_eq!(
                        Some(origin.source),
                        tree.program_lookup(provider)
                            .unwrap()
                            .source
                            .unwrap()
                            .identity
                    );
                    assert_eq!(
                        origin.span.start,
                        case["sources"][provider]
                            .as_str()
                            .unwrap()
                            .find(case["selected"].as_str().unwrap())
                            .unwrap() as u32,
                        "{}",
                        case["name"]
                    );
                    if let Some(row) = origin.declaration {
                        assert_eq!(
                            lookup.symbol_by_id(row).unwrap().file_path.as_ref(),
                            provider
                        );
                    }
                }
                let file = &files[main];
                let lookup = FileLookup::for_file(tree, file, &ids);
                let model = SemanticModel::production();
                let mut seen = FxHashMap::default();
                let ctx = FileContext {
                    file_path: file.path.clone(),
                    language: "typescript".into(),
                    imports: vec![],
                    file_namespace: None,
                };
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
                    if !targets.contains_key(&selector) {
                        continue;
                    }
                    let mut site = testkit::ref_ctx(
                        reference,
                        &file.symbols[reference.source_symbol_index],
                        vec![],
                    );
                    site.source_symbol_id = ids.row_id(&file.path, reference.source_symbol_index);
                    lookup.set_cursor(reference.byte_offset);
                    let SolveOutcome::Resolved(info) = model.get_symbol_info(
                        &site,
                        &ctx,
                        &lookup,
                        &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
                    ) else {
                        panic!(
                            "{}: constructor cascade stopped at {selector}",
                            case["name"]
                        );
                    };
                    if let Some(ty) = info.resolved_yield_type {
                        lookup.record_rhs_type(index, "poison", ty);
                    }
                    seen.insert(selector, info.target_symbol_id);
                }
                assert_eq!(
                    seen, targets,
                    "{}: exact downstream declarations",
                    case["name"]
                );
            };
            let tree = Compilation::build_with_context(
                &files,
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
            if case["name"]
                .as_str()
                .unwrap()
                .starts_with("inherited_merged_provider")
            {
                config.programs.as_mut().unwrap()[0].source_binding_order = None;
                let tree = Compilation::build_with_context(
                    &files,
                    &ids,
                    Arc::clone(&arena),
                    Some(&config),
                    &HashSet::new(),
                );
                let without_order = |tree: &Compilation| {
                    let lookup = tree.program_lookup("main.ts").unwrap();
                    let call = lookup
                        .source
                        .unwrap()
                        .constructor_calls
                        .values()
                        .find_map(Option::as_ref)
                        .unwrap();
                    assert!(call.selected.is_none(), "inherited provider order must be attested even when signature origins share a source");
                    assert_eq!(
                        Some(call.return_type),
                        lookup.field_type_id_of(expected.unwrap())
                    );
                };
                without_order(&tree);
                tree.persist_type_info(db.conn()).unwrap();
                let restored = Arc::new(TypeArena::new());
                restored.restore_snapshot(&arena.serialize_snapshot());
                let mut cold = Compilation::build(&[], &Default::default(), restored);
                cold.ingest_from_db(db.conn());
                without_order(&cold);
            }
        }
    }
}

#[test]
fn constructor_origins_follow_configuration_edits_deletion_and_consensus() {
    for same_result in [false, true] {
        let arena = Arc::new(TypeArena::new());
        let right = if same_result {
            "interface Factory { new(value: number): First; }"
        } else {
            "interface Second { other(): void } interface Factory { new(value: number): Second; }"
        };
        let files = parse(&arena, &[("a.ts", "interface First { touch(): void } interface Factory { new(value: number): First; }"),
            ("z.ts", right), ("main.ts", "declare const Build: Factory; const actual = new Build(1);")]);
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &files,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let first = owner(&ids, &files[0], "First");
        let second = if same_result {
            first
        } else {
            owner(&ids, &files[1], "Second")
        };
        let mut config = context(&files.iter().collect::<Vec<_>>());
        config.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
            strict_parameters: true,
            strict_nulls: true,
            bivariant_methods: Some(true),
        });
        let check = |tree: &Compilation, result: Option<i64>, selected: Option<&str>| {
            let lookup = tree.program_lookup("main.ts").unwrap();
            let start = files[2].content.as_ref().unwrap().find("actual =").unwrap() as u32;
            let pair = lookup.source.and_then(|source| {
                source
                    .initializers
                    .iter()
                    .find(|(id, _)| id.0.start == start)
            });
            let actual = pair.and_then(|(_, ty)| *ty);
            assert_eq!(
                actual.and_then(|ty| head_decl_id(tree.type_arena().unwrap(), ty)),
                result
            );
            let call = pair
                .and_then(|(id, _)| lookup.source.unwrap().constructor_calls.get(id))
                .and_then(Option::as_ref);
            if result.is_none() {
                assert!(call.is_none());
                return;
            }
            let call = call.unwrap();
            assert_eq!(Some(call.return_type), actual);
            if let Some(path) = selected {
                let origin = call.origins[call.selected.unwrap()].as_ref().unwrap();
                assert_eq!(
                    Some(origin.source),
                    tree.program_lookup(path).unwrap().source.unwrap().identity
                );
            } else {
                assert!(
                    call.selected.is_none(),
                    "consensus is not a selected source signature"
                );
            }
        };
        let tree = Compilation::build_with_context(
            &files,
            &ids,
            Arc::clone(&arena),
            Some(&config),
            &HashSet::new(),
        );
        check(&tree, same_result.then_some(first), None);
        tree.persist_type_info(db.conn()).unwrap();
        for (order, result, selected) in [
            (
                Some(vec!["z.ts", "a.ts", "main.ts"]),
                Some(first),
                Some("a.ts"),
            ),
            (
                Some(vec!["a.ts", "z.ts", "main.ts"]),
                Some(second),
                Some("z.ts"),
            ),
            (
                Some(vec!["z.ts", "z.ts", "main.ts"]),
                same_result.then_some(first),
                None,
            ),
            (None, same_result.then_some(first), None),
        ] {
            config.programs.as_mut().unwrap()[0].source_binding_order =
                order.map(|paths| paths.into_iter().map(String::from).collect());
            let mut changed = Compilation::build_with_context(
                &[],
                &Default::default(),
                Arc::clone(&arena),
                Some(&config),
                &HashSet::new(),
            );
            changed.ingest_from_db(db.conn());
            check(&changed, result, selected);
            changed.persist_type_info(db.conn()).unwrap();
            let restored = Arc::new(TypeArena::new());
            restored.restore_snapshot(&arena.serialize_snapshot());
            let mut cold = Compilation::build(&[], &Default::default(), restored);
            cold.ingest_from_db(db.conn());
            check(&cold, result, selected);
        }
        let edited = parse(
            &arena,
            &[("z.ts", &right.replace("value: number", "value: string"))],
        );
        let (_, edited_ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &edited,
            "internal",
            Some(&arena),
        )
        .unwrap();
        config.programs.as_mut().unwrap()[0].sources[1].content_hash =
            edited[0].content_hash.clone();
        let mut changed = Compilation::build_with_context(
            &edited,
            &edited_ids,
            Arc::clone(&arena),
            Some(&config),
            &HashSet::new(),
        );
        changed.ingest_from_db(db.conn());
        check(&changed, Some(first), Some("a.ts"));
        changed.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &Default::default(), Arc::clone(&restored));
        cold.ingest_from_db(db.conn());
        check(&cold, Some(first), Some("a.ts"));
        db.conn()
            .execute("DELETE FROM files WHERE path='z.ts'", [])
            .unwrap();
        let mut deleted = Compilation::build(&[], &Default::default(), Arc::clone(&restored));
        deleted.ingest_from_db(db.conn());
        check(&deleted, None, None);
        config.programs.as_mut().unwrap()[0]
            .sources
            .retain(|source| source.path != "z.ts");
        let mut recovered = Compilation::build_with_context(
            &[],
            &Default::default(),
            restored,
            Some(&config),
            &HashSet::new(),
        );
        recovered.ingest_from_db(db.conn());
        check(&recovered, Some(first), Some("a.ts"));
    }
}
