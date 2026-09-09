use super::*;
use crate::indexer::{
    programs::{Program, ProgramSource, SourceScope},
    resolve::{engine::compilation::Compilation, ProjectContext},
};
use crate::type_checker::core::types::{
    LitValue, MappedModifier, TypeOperator as Op, TypeProperty,
};
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};

fn parse(arena: &TypeArena, sources: &[(&str, &str)]) -> Vec<ParsedFile> {
    let directory = tempfile::tempdir().unwrap();
    sources
        .iter()
        .map(|(path, source)| {
            let absolute_path = directory.path().join(path);
            std::fs::write(&absolute_path, source).unwrap();
            crate::indexer::parse_file::parse_file_with_arena(
                &crate::walker::WalkedFile {
                    relative_path: (*path).into(),
                    absolute_path,
                    language: "typescript",
                },
                crate::languages::default_registry(),
                arena,
            )
            .unwrap()
        })
        .collect()
}
fn config(files: &[&ParsedFile]) -> ProjectContext {
    ProjectContext {
        programs: Some(vec![Program {
            key: "only".into(),
            fingerprint: "only".into(),
            complete: true,
            callable_policy: None,
            compiler_intrinsics: None,
            source_binding_order: None,
            sources: files
                .iter()
                .map(|f| ProgramSource {
                    path: f.path.clone(),
                    content_hash: f.content_hash.clone(),
                    scope: SourceScope::Syntax,
                })
                .collect(),
        }]),
        ..Default::default()
    }
}

fn expected(
    shape: &Value,
    arena: &TypeArena,
    outer: &[TypeId],
    mapped: &[TypeId],
    cursor: &mut usize,
) -> TypeId {
    let modifier = |v: &Value| match v.as_str().unwrap() {
        "preserve" => MappedModifier::Preserve,
        "add" => MappedModifier::Add,
        "remove" => MappedModifier::Remove,
        _ => panic!(),
    };
    arena.intern(match shape[0].as_str().unwrap() {
        "param" => {
            let name = shape[1].as_str().unwrap();
            let index = shape[2].as_u64().unwrap() as usize;
            return if name == "outer" {
                outer[index]
            } else {
                assert_eq!(index, 0);
                mapped[name
                    .strip_prefix("mapped")
                    .unwrap()
                    .parse::<usize>()
                    .unwrap()]
            };
        }
        "string" => Type::Literal(LitValue::Str(shape[1].as_str().unwrap().into())),
        "keyof" => Type::Operator(Op::KeyOf(expected(&shape[1], arena, outer, mapped, cursor))),
        "index" => Type::Operator(Op::IndexedAccess {
            object: expected(&shape[1], arena, outer, mapped, cursor),
            index: expected(&shape[2], arena, outer, mapped, cursor),
        }),
        "intersection" => Type::Intersection(
            shape.as_array().unwrap()[1..]
                .iter()
                .map(|s| expected(s, arena, outer, mapped, cursor))
                .collect(),
        ),
        "object" => Type::Operator(Op::Object(
            shape.as_array().unwrap()[1..]
                .iter()
                .map(|p| TypeProperty {
                    key: arena.intern(Type::Literal(LitValue::Str(p[0].as_str().unwrap().into()))),
                    value: expected(&p[3], arena, outer, mapped, cursor),
                    optional: p[1].as_bool().unwrap(),
                    readonly: p[2].as_bool().unwrap(),
                    index: false,
                })
                .collect(),
        )),
        "mapped" => {
            let parameter = mapped[*cursor];
            *cursor += 1;
            Type::Operator(Op::Mapped {
                parameter,
                readonly: modifier(&shape[1]),
                optional: modifier(&shape[2]),
                keys: expected(&shape[3], arena, outer, mapped, cursor),
                remap: (!shape[4].is_null())
                    .then(|| expected(&shape[4], arena, outer, mapped, cursor)),
                value: expected(&shape[5], arena, outer, mapped, cursor),
            })
        }
        _ => panic!("{shape}"),
    })
}

#[test]
fn configured_structural_recipes_keep_binders_through_filtered_portable_poisoned_and_cold_inputs() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/structural_type_fixtures.json"
    ))
    .unwrap();
    let sources: Vec<_> = cases
        .iter()
        .enumerate()
        .map(|(i, c)| {
            (
                format!("case{i}.ts"),
                format!(
                    "export interface Shape<T, K extends keyof T> {{ value: {}; }}",
                    c["syntax"].as_str().unwrap()
                ),
            )
        })
        .collect();
    let original = TypeArena::new();
    let providers = parse(
        &original,
        &sources
            .iter()
            .map(|(p, s)| (p.as_str(), s.as_str()))
            .collect::<Vec<_>>(),
    );
    let arena = Arc::new(TypeArena::new());
    arena.class("shift portable IDs");
    let mut files: Vec<_> = providers
        .into_iter()
        .map(|mut file| {
            reduce_to_contract(&mut file);
            let payload =
                serde_json::to_string(&CachedParse::from_parsed(&file, &original)).unwrap();
            let cached: CachedParse = serde_json::from_str(&payload).unwrap();
            let mut restored =
                cached.into_parsed(&arena, &file.path, &file.content_hash, file.size, None);
            restored.content = file.content;
            reduce_to_contract(&mut restored);
            restored
        })
        .collect();
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "external",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let arena = tree.type_arena().unwrap();
        let mut all_binders = HashSet::new();
        for (file, case) in files.iter().zip(&cases) {
            let lookup = tree.program_lookup(&file.path).unwrap();
            let graph = file.flow.lexical.as_ref().unwrap();
            let owner = ids
                .row_id(
                    &file.path,
                    file.symbols.iter().position(|s| s.name == "Shape").unwrap(),
                )
                .unwrap();
            let outer: Vec<_> = lookup
                .canonical_type_info(owner)
                .unwrap()
                .generic_param_ids
                .iter()
                .map(|&p| arena.generic_type(p))
                .collect();
            let mut mapped: Vec<_> = graph
                .types
                .signatures
                .iter()
                .filter(|s| s.declaration.is_none() && s.generics.len() == 1)
                .map(|s| s.id)
                .collect();
            mapped.sort_by_key(|id| id.0.start);
            let mapped: Vec<_> = mapped
                .iter()
                .map(|&s| arena.generic_type(lookup.signature(s).unwrap().generic_parameters[0]))
                .collect();
            for &id in &mapped {
                assert!(all_binders.insert(id));
                assert!(!outer.contains(&id));
            }
            let value = graph
                .types
                .signatures
                .iter()
                .find(|s| {
                    s.id.0.start == file.content.as_ref().unwrap().find("value:").unwrap() as u32
                })
                .unwrap()
                .id;
            assert_eq!(
                lookup.signature(value).unwrap().result,
                Some(expected(&case["shape"], arena, &outer, &mapped, &mut 0)),
                "{}",
                case["name"]
            );
        }
    };
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
    for file in &mut files {
        let input = serde_json::to_value(capture(file, &ids, &tree, &arena)).unwrap();
        for symbol in &mut file.symbols {
            symbol.name = "poisoned".into();
            symbol.qualified_name = "poisoned".into();
            symbol.signature = None;
        }
        assert_eq!(
            serde_json::to_value(capture(file, &ids, &tree, &arena)).unwrap(),
            input
        );
    }
}

#[test]
fn imported_structural_operands_retarget_and_do_not_revive_deleted_namesakes() {
    let arena = Arc::new(TypeArena::new());
    let source = "import type { Doc } from './barrel'; export function read(): { [P in keyof Doc]: Doc[P] } { throw 0; }";
    let files = parse(
        &arena,
        &[
            ("left.ts", "export interface Doc { left: number; }"),
            ("right.ts", "export interface Doc { right: string; }"),
            ("barrel.ts", "export { Doc } from './left';"),
            ("main.ts", source),
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
    let row = ids
        .row_id(
            "main.ts",
            files[3]
                .symbols
                .iter()
                .position(|s| s.name == "read")
                .unwrap(),
        )
        .unwrap();
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    let check = |tree: &Compilation, provider: Option<usize>| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        let arena = tree.type_arena().unwrap();
        let Type::Operator(Op::Mapped {
            parameter,
            keys,
            value,
            ..
        }) = arena.get(lookup.return_type_id_of(row).unwrap())
        else {
            panic!("mapped type")
        };
        let Type::Operator(Op::KeyOf(head)) = arena.get(keys) else {
            panic!("key domain")
        };
        assert_eq!(
            arena.get(value),
            Type::Operator(Op::IndexedAccess {
                object: head,
                index: parameter
            })
        );
        match provider {
            Some(i) => assert!(
                matches!(arena.get(head), Type::Decl { symbol_id, .. } if Some(symbol_id) == ids.row_id(&files[i].path, 0))
            ),
            None => assert_eq!(arena.get(head), Type::Unknown),
        }
        assert!(matches!(arena.get(parameter), Type::Generic { .. }));
    };
    check(&tree, Some(0));
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(&arena, &[("barrel.ts", "export { Doc } from './right';")]);
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let updated = config(&[&files[0], &files[1], &changed[0], &files[3]]);
    let mut edited = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&updated),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, Some(1));
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, Some(1));
    db.conn()
        .execute("DELETE FROM files WHERE path='right.ts'", [])
        .unwrap();
    let remaining = config(&[&files[0], &changed[0], &files[3]]);
    let mut deleted = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        restored,
        Some(&remaining),
        &HashSet::new(),
    );
    deleted.ingest_from_db(db.conn());
    check(&deleted, None);
}
