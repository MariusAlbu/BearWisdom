use super::*;
use crate::indexer::external_parse_payload::CachedParse;
use crate::indexer::resolve::engine::contract::FlowCacheLookup;
use crate::indexer::resolve::engine::program_view::merge_proof::tests::{context, owner, parse};
use crate::type_checker::core::types::{Intrinsic, LitValue, Type};
use std::{collections::HashSet, sync::Arc};

#[test]
fn compiler_cross_source_order_survives_portable_poisoned_and_cold_views() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/overload_order_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        let sources: Vec<_> = case["sources"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(path, source)| (path.as_str(), source.as_str().unwrap()))
            .collect();
        let original = TypeArena::new();
        let parsed = parse(&original, &sources);
        let arena = Arc::new(TypeArena::new());
        for n in 0..32 {
            arena.intern(Type::Literal(LitValue::Int(n)));
        }
        let mut files: Vec<_> = parsed
            .iter()
            .map(|input| {
                let payload =
                    serde_json::to_string(&CachedParse::from_parsed(input, &original)).unwrap();
                let cached: CachedParse = serde_json::from_str(&payload).unwrap();
                let mut file =
                    cached.into_parsed(&arena, &input.path, &input.content_hash, input.size, None);
                file.content = input.content.clone();
                crate::indexer::contract_filter::reduce_to_contract(&mut file);
                file
            })
            .collect();
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &files,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let main = files.iter().position(|f| f.path == "main.ts").unwrap();
        let api = owner(&ids, &files[main], "api");
        let target_file = case["selectedFile"].as_str().unwrap();
        let expected = case["sources"][target_file]
            .as_str()
            .unwrap()
            .find(case["selected"].as_str().unwrap())
            .unwrap() as u32;
        for file in &mut files {
            for symbol in &mut file.symbols {
                symbol.name = "poison".into();
                symbol.qualified_name = "poison.display".into();
                symbol.signature = None;
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
        let check = |tree: &Compilation| {
            let lookup = tree.program_lookup("main.ts").unwrap();
            let arena = tree.type_arena().unwrap();
            let name = lookup.member_index().unwrap().name("pick").unwrap();
            let file = crate::indexer::resolve::engine::file_lookup::FileLookup::for_file(
                tree,
                &files[main],
                &ids,
            );
            let selector = case["sources"]["main.ts"]
                .as_str()
                .unwrap()
                .find("api.pick")
                .unwrap() as u32
                + 4;
            let arguments = file.source_call_arguments(selector).unwrap().unwrap();
            let actual = crate::indexer::resolve::engine::arg_types::resolve_arg_types(
                &file, arena, arguments,
            );
            let receiver = lookup.field_type_id_of(api).unwrap();
            let call = super::super::select(&lookup, receiver, name, &actual, &[])
                .unwrap()
                .unwrap_or_else(|_| {
                    let owner =
                        crate::indexer::resolve::engine::head_decl::head_decl_id(arena, receiver)
                            .unwrap();
                    let members: Vec<_> = lookup
                        .nominal_surface(owner)
                        .unwrap()
                        .members
                        .iter()
                        .filter(|m| m.origin.name == Some(name))
                        .collect();
                    panic!(
                        "{}: order={:?}, ranks={:?}, origins={:?}, syntax={:?}",
                        case["name"],
                        candidates(&lookup, &members),
                        lookup.view.source_binding_order,
                        members.iter().map(|m| &m.origin).collect::<Vec<_>>(),
                        members
                            .iter()
                            .map(|m| &m.signature.syntax.ordering)
                            .collect::<Vec<_>>()
                    );
                });
            let origin = &call.origins[call.selected];
            assert_eq!(origin.span.start, expected, "{}", case["name"]);
            assert_eq!(
                lookup
                    .symbol_by_id(origin.declaration.unwrap())
                    .unwrap()
                    .file_path
                    .as_ref(),
                target_file
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
    }
}

#[test]
fn changing_only_binding_order_retargets_without_row_or_source_changes() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            ("a.ts", "interface Factory { pick(value: number): string; }"),
            ("z.ts", "interface Factory { pick(value: number): number; }"),
            ("main.ts", "declare const api: Factory;"),
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
    let api = owner(&ids, &files[2], "api");
    let mut config = context(&files.iter().collect::<Vec<_>>());
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    tree.persist_type_info(db.conn()).unwrap();
    for (order, expected) in [
        (None, None),
        (
            Some(vec!["z.ts", "a.ts", "main.ts"]),
            Some(Intrinsic::String),
        ),
        (
            Some(vec!["a.ts", "z.ts", "main.ts"]),
            Some(Intrinsic::Number),
        ),
        (Some(vec!["z.ts", "z.ts", "main.ts"]), None),
        (None, None),
    ] {
        config.programs.as_mut().unwrap()[0].source_binding_order =
            order.map(|paths| paths.into_iter().map(String::from).collect());
        let mut tree = Compilation::build_with_context(
            &[],
            &Default::default(),
            Arc::clone(&arena),
            Some(&config),
            &HashSet::new(),
        );
        tree.ingest_from_db(db.conn());
        let lookup = tree.program_lookup("main.ts").unwrap();
        let receiver = lookup.field_type_id_of(api).unwrap();
        let name = lookup.member_index().unwrap().name("pick").unwrap();
        let call = super::super::select(
            &lookup,
            receiver,
            name,
            &[arena.intern(Type::Literal(LitValue::Int(42)))],
            &[],
        )
        .unwrap();
        match expected {
            Some(kind) => assert_eq!(arena.get(call.unwrap().return_type), Type::Intrinsic(kind)),
            None => assert!(call.is_err()),
        }
        tree.persist_type_info(db.conn()).unwrap();
    }
}

#[test]
fn later_merged_groups_precede_earlier_groups_but_keep_each_groups_order() {
    assert_eq!(
        reorder(&[(1, 1, false), (1, 1, false), (1, 2, false), (1, 2, false)]),
        [2, 3, 0, 1]
    );
}

#[test]
fn inherited_symbols_keep_order_and_specializations_move_first_stably() {
    assert_eq!(
        reorder(&[
            (1, 1, false),
            (1, 1, true),
            (1, 2, false),
            (1, 2, true),
            (2, 3, false),
            (2, 3, true)
        ]),
        [1, 3, 5, 2, 0, 4]
    );
}
