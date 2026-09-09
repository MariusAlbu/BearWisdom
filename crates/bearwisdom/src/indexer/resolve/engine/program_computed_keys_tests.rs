use super::*;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::ProjectContext;
use crate::indexer::{
    programs::{Program, ProgramSource, SourceScope},
    symbol_ids::SymbolIds,
};
use std::{collections::HashSet, sync::Arc};

#[path = "program_value_domain_tests.rs"]
mod value_domain;

fn parse(arena: &TypeArena, sources: &[(&str, &str)]) -> Vec<ParsedFile> {
    let directory = tempfile::tempdir().unwrap();
    let registry = crate::languages::default_registry();
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
                &registry,
                arena,
            )
            .unwrap()
        })
        .collect()
}
fn context(files: &[&ParsedFile]) -> ProjectContext {
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
fn key(lookup: &Lookup, source: &str, expression: &str) -> Option<Option<TypeId>> {
    let start = source.find(expression).unwrap() as u32;
    lookup.computed_key(SourceSpan {
        start,
        end: start + expression.len() as u32,
    })
}
fn unique(lookup: &Lookup, source: &str, declaration: &str) -> TypeId {
    use crate::indexer::resolve::engine::contract::FlowCacheLookup;
    let start = source.find(declaration).unwrap() as u32;
    lookup
        .source_unique_symbol(SourceSpan {
            start,
            end: start + declaration.len() as u32,
        })
        .unwrap_or_else(|| panic!("missing unique declaration {declaration} in {source}"))
}

#[test]
fn computed_unique_keys_bind_local_global_nested_and_static_member_ids() {
    let arena = Arc::new(TypeArena::new());
    let provider = "interface Keys { readonly token: unique symbol; } declare const keys: Keys;";
    let source = "export {}; declare const tag: unique symbol; declare class Factory { static readonly token: unique symbol; } interface Box<T> { value: T; } declare const box: Box<Keys>; interface Uses { [tag](): void; [keys.token](): void; [box.value.token](): void; [Factory.token](): void; }";
    let files = parse(&arena, &[("provider.d.ts", provider), ("main.ts", source)]);
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
    let lookup = tree.program_lookup("main.ts").unwrap();
    let global = tree.program_lookup("provider.d.ts").unwrap();
    assert_eq!(
        key(&lookup, source, "[tag]"),
        Some(Some(unique(&lookup, source, "tag: unique symbol")))
    );
    let token = unique(&global, provider, "readonly token: unique symbol");
    assert_eq!(key(&lookup, source, "[keys.token]"), Some(Some(token)));
    assert_eq!(key(&lookup, source, "[box.value.token]"), Some(Some(token)));
    assert_eq!(
        key(&lookup, source, "[Factory.token]"),
        Some(Some(unique(
            &lookup,
            source,
            "static readonly token: unique symbol"
        )))
    );
}

#[test]
fn computed_unique_keys_do_not_guess_plain_optional_private_duplicate_or_shadowed_values() {
    let arena = Arc::new(TypeArena::new());
    let source = "export {}; declare const tag: symbol; interface Keys { readonly optional?: unique symbol; readonly duplicate: unique symbol; readonly duplicate: symbol; } declare const keys: Keys; declare class Factory { private static readonly hidden: unique symbol; static readonly token: unique symbol; } declare const instance: Factory; interface Uses { [tag](): void; [keys.optional](): void; [keys.duplicate](): void; [Factory.hidden](): void; [instance.token](): void; [missing](): void; } function local(keys: symbol) { interface Shadow { [keys.optional](): void; } }";
    let files = parse(&arena, &[("main.ts", source)]);
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
        Some(&context(&[&files[0]])),
        &HashSet::new(),
    );
    let lookup = tree.program_lookup("main.ts").unwrap();
    for expression in [
        "[tag]",
        "[keys.optional]",
        "[keys.duplicate]",
        "[Factory.hidden]",
        "[instance.token]",
        "[missing]",
    ] {
        assert_eq!(key(&lookup, source, expression), Some(None), "{expression}");
    }
    let start = source.rfind("[keys.optional]").unwrap() as u32;
    assert_eq!(
        lookup.computed_key(SourceSpan {
            start,
            end: start + 15
        }),
        Some(None)
    );
}

#[test]
fn computed_unique_keys_retarget_import_ids_after_edits_and_deletion_without_recapturing_consumer()
{
    let arena = Arc::new(TypeArena::new());
    let source =
        "import { tag as selected } from './barrel'; export interface Uses { [selected](): void; }";
    let provider = "export declare const tag: unique symbol;";
    let files = parse(
        &arena,
        &[
            ("left.ts", provider),
            ("right.ts", provider),
            ("barrel.ts", "export { tag } from './left';"),
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
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    let check = |tree: &Compilation, expected: &str, unexpected: &str| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        let wanted = unique(
            &tree.program_lookup(expected).unwrap(),
            provider,
            "tag: unique symbol",
        );
        let other = unique(
            &tree.program_lookup(unexpected).unwrap(),
            provider,
            "tag: unique symbol",
        );
        assert_ne!(
            wanted, other,
            "identical source bytes in different files are different origins"
        );
        assert_eq!(key(&lookup, source, "[selected]"), Some(Some(wanted)));
    };
    check(&tree, "left.ts", "right.ts");
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(&arena, &[("barrel.ts", "export { tag } from './right';")]);
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let updated = context(&[&files[0], &files[1], &changed[0], &files[3]]);
    let mut edited = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&updated),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, "right.ts", "left.ts");
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, "right.ts", "left.ts");
    db.conn()
        .execute("DELETE FROM files WHERE path='right.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), restored);
    deleted.ingest_from_db(db.conn());
    assert!(key(
        &deleted.program_lookup("main.ts").unwrap(),
        source,
        "[selected]"
    )
    .flatten()
    .is_none());
}

#[test]
fn computed_unique_keys_preserve_rowless_member_sources_through_portable_and_cold_inputs() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let original = TypeArena::new();
    let source = "function discard() {} interface Keys { readonly token: unique symbol; } declare const keys: Keys; interface Uses { [keys.token](): void; }";
    let mut files = parse(&original, &[("provider.d.ts", source)]);
    let discarded = files[0]
        .symbols
        .iter()
        .position(|s| s.name == "discard")
        .unwrap();
    let property = files[0]
        .symbols
        .iter()
        .position(|s| s.name == "token")
        .unwrap();
    files[0].symbols[property].parent_index = Some(discarded);
    reduce_to_contract(&mut files[0]);
    assert!(!files[0].symbols.iter().any(|s| s.name == "token"));
    let payload = serde_json::to_string(&CachedParse::from_parsed(&files[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut file = cached.into_parsed(
        &arena,
        "provider.d.ts",
        &files[0].content_hash,
        files[0].size,
        None,
    );
    file.content = Some(source.into());
    reduce_to_contract(&mut file);
    assert!(!file.symbols.iter().any(|s| s.name == "token"));
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&file),
        "external",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&file])),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("provider.d.ts").unwrap();
        let token = unique(&lookup, source, "readonly token: unique symbol");
        assert_eq!(key(&lookup, source, "[keys.token]"), Some(Some(token)));
    };
    check(&tree);
    let before = serde_json::to_value(program_types::capture(&file, &ids, &tree, &arena)).unwrap();
    for symbol in &mut file.symbols {
        symbol.name = "poison".into();
        symbol.qualified_name = "poison.display".into();
        symbol.signature = None;
    }
    assert_eq!(
        serde_json::to_value(program_types::capture(&file, &ids, &tree, &arena)).unwrap(),
        before
    );
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
}

#[test]
fn computed_unique_keys_reject_unproved_merged_owners_and_preserve_program_isolation() {
    let arena = Arc::new(TypeArena::new());
    let source = "declare const tag: unique symbol; interface Uses { [tag](): void; }";
    let files = parse(&arena, &[("left.ts", source), ("right.ts", source)]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut config = context(&[&files[0]]);
    let mut other = context(&[&files[1]]).programs.unwrap().remove(0);
    other.key = "other".into();
    other.fingerprint = "other".into();
    config.programs.as_mut().unwrap().push(other);
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    let left = tree.program_lookup("left.ts").unwrap();
    let right = tree.program_lookup("right.ts").unwrap();
    let a = key(&left, source, "[tag]").flatten().unwrap();
    let b = key(&right, source, "[tag]").flatten().unwrap();
    assert_ne!(a, b);
    assert!(!(&left as &dyn SymbolLookup).accepts_type_context(&arena, b));
    assert!(!(&right as &dyn SymbolLookup).accepts_type_context(&arena, a));
    let merged_source = "interface Keys { readonly token: unique symbol; } interface Keys { readonly token: unique symbol; } declare const keys: Keys; interface Uses { [keys.token](): void; }";
    let merged = parse(&arena, &[("merged.ts", merged_source)]);
    let (_, merged_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &merged,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build_with_context(
        &merged,
        &merged_ids,
        arena,
        Some(&context(&[&merged[0]])),
        &HashSet::new(),
    );
    assert_eq!(
        key(
            &tree.program_lookup("merged.ts").unwrap(),
            merged_source,
            "[keys.token]"
        ),
        Some(None)
    );
}

#[test]
fn computed_unique_keys_match_independent_compiler_declaration_targets() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/computed_unique_key_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        let arena = Arc::new(TypeArena::new());
        let sources: Vec<_> = case["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| (f["path"].as_str().unwrap(), f["source"].as_str().unwrap()))
            .collect();
        let files = parse(&arena, &sources);
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
        let check = |tree: &Compilation| {
            for expected in case["keys"].as_array().unwrap() {
                let path = expected["path"].as_str().unwrap();
                let source = sources.iter().find(|s| s.0 == path).unwrap().1;
                if expected["unbound"] == true {
                    assert_eq!(
                        key(
                            &tree.program_lookup(path).unwrap(),
                            source,
                            expected["expression"].as_str().unwrap()
                        ),
                        Some(None),
                        "{source}"
                    );
                    continue;
                }
                let owner_path = expected["ownerPath"].as_str().unwrap();
                let owner_source = sources.iter().find(|s| s.0 == owner_path).unwrap().1;
                let target = unique(
                    &tree.program_lookup(owner_path).unwrap(),
                    owner_source,
                    expected["owner"].as_str().unwrap(),
                );
                assert_eq!(
                    key(
                        &tree.program_lookup(path).unwrap(),
                        source,
                        expected["expression"].as_str().unwrap()
                    ),
                    Some(Some(target))
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
    }
}

#[test]
fn source_value_queries_bind_forward_annotation_dependencies_to_exact_unique_origins() {
    let arena = Arc::new(TypeArena::new());
    let source = "export {}; declare const first: typeof second; declare const second: typeof original; declare const original: unique symbol; interface Uses { [first](): void; }";
    let files = parse(&arena, &[("main.ts", source)]);
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
        arena,
        Some(&context(&[&files[0]])),
        &HashSet::new(),
    );
    let lookup = tree.program_lookup("main.ts").unwrap();
    assert_eq!(
        key(&lookup, source, "[first]"),
        Some(Some(unique(&lookup, source, "original: unique symbol")))
    );
}

#[test]
fn source_value_queries_match_compiler_targets_fresh_cold_and_without_display_metadata() {
    use crate::indexer::resolve::engine::contract::FlowCacheLookup;
    let mut cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/value_query_fixtures.json"
    ))
    .unwrap();
    cases.extend(
        serde_json::from_str::<Vec<serde_json::Value>>(include_str!(
            "../../../resolution_oracle/value_domain_fixtures.json"
        ))
        .unwrap(),
    );
    cases.extend(
        serde_json::from_str::<Vec<serde_json::Value>>(include_str!(
            "../../../resolution_oracle/interface_query_fixtures.json"
        ))
        .unwrap(),
    );
    for case in cases {
        let arena = Arc::new(TypeArena::new());
        let sources: Vec<_> = case["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| (f["path"].as_str().unwrap(), f["source"].as_str().unwrap()))
            .collect();
        let mut files = parse(&arena, &sources);
        for file in &files {
            assert!(
                file.flow.lexical.as_ref().unwrap().module.complete,
                "incomplete source module: {:?}",
                file.content
            );
        }
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &files,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let query_rows: Vec<_> = case["queryTypes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|expected| {
                let path = expected["path"].as_str().unwrap();
                let file = files.iter().find(|f| f.path == path).unwrap();
                let slot = file
                    .symbols
                    .iter()
                    .position(|s| Some(s.name.as_str()) == expected["binding"].as_str())
                    .unwrap();
                ids.row_id(path, slot).unwrap()
            })
            .collect();
        let tree = Compilation::build_with_context(
            &files,
            &ids,
            Arc::clone(&arena),
            Some(&context(&files.iter().collect::<Vec<_>>())),
            &HashSet::new(),
        );
        let check = |tree: &Compilation| {
            for expected in case["keys"].as_array().unwrap() {
                let path = expected["path"].as_str().unwrap();
                let source = sources.iter().find(|s| s.0 == path).unwrap().1;
                if expected["unbound"] == true {
                    assert_eq!(
                        key(
                            &tree.program_lookup(path).unwrap(),
                            source,
                            expected["expression"].as_str().unwrap()
                        ),
                        Some(None),
                        "{source}"
                    );
                    continue;
                }
                let owner_path = expected["ownerPath"].as_str().unwrap();
                let owner_source = sources.iter().find(|s| s.0 == owner_path).unwrap().1;
                let target = unique(
                    &tree.program_lookup(owner_path).unwrap(),
                    owner_source,
                    expected["owner"].as_str().unwrap(),
                );
                let wanted = if expected["unsupported"] == true {
                    None
                } else {
                    Some(target)
                };
                let lookup = tree.program_lookup(path).unwrap();
                let actual = key(&lookup, source, expected["expression"].as_str().unwrap());
                assert_eq!(actual, Some(wanted), "{source}");
            }
            for (expected, &row) in case["queryTypes"]
                .as_array()
                .into_iter()
                .flatten()
                .zip(&query_rows)
            {
                let path = expected["path"].as_str().unwrap();
                let expression = expected["expression"].as_str().unwrap();
                let source = sources.iter().find(|s| s.0 == path).unwrap().1;
                let lookup = tree.program_lookup(path).unwrap();
                let start = source.find(expression).unwrap() as u32;
                let ty = lookup
                    .source_value_type(SourceSpan {
                        start,
                        end: start + expression.len() as u32,
                    })
                    .flatten()
                    .unwrap();
                let intrinsic = serde_json::from_value(expected["intrinsic"].clone()).unwrap();
                assert_eq!(
                    tree.type_arena().unwrap().get(ty),
                    Type::Intrinsic(intrinsic)
                );
                assert_eq!(lookup.field_type_id_of(row), Some(ty));
            }
        };
        check(&tree);
        for file in &mut files {
            let before =
                serde_json::to_value(program_types::capture(file, &ids, &tree, &arena)).unwrap();
            for symbol in &mut file.symbols {
                symbol.name = "poison".into();
                symbol.qualified_name = "poison.display".into();
                symbol.signature = None;
            }
            for recipe in file
                .flow
                .lexical
                .as_mut()
                .unwrap()
                .types
                .annotations
                .values_mut()
            {
                if let crate::indexer::lexical::type_syntax::TypeExpr::ValueQuery {
                    legacy, ..
                } = recipe
                {
                    *legacy = "invalid display".into();
                }
            }
            assert_eq!(
                serde_json::to_value(program_types::capture(file, &ids, &tree, &arena)).unwrap(),
                before
            );
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
fn source_value_queries_reject_cycles_missing_and_ambiguous_namespace_providers() {
    use crate::indexer::resolve::engine::contract::FlowCacheLookup;
    let arena = Arc::new(TypeArena::new());
    let source = "import * as provider from './barrel'; declare const cyclic: typeof other; declare const other: typeof cyclic; declare const missing: typeof absent; declare const ambiguous: typeof provider.tag; export interface Uses { [cyclic](): void; [missing](): void; [ambiguous](): void; }";
    let files = parse(
        &arena,
        &[
            ("left.ts", "export declare const tag: unique symbol;"),
            ("right.ts", "export declare const tag: unique symbol;"),
            (
                "barrel.ts",
                "export * from './left'; export * from './right';",
            ),
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
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        for expression in ["[cyclic]", "[missing]", "[ambiguous]"] {
            assert_eq!(key(&lookup, source, expression), Some(None));
        }
        for query in [
            "typeof other",
            "typeof cyclic",
            "typeof absent",
            "typeof provider.tag",
        ] {
            let start = source.find(query).unwrap() as u32;
            let value = lookup.source_value_type(SourceSpan {
                start,
                end: start + query.len() as u32,
            });
            assert!(value.is_some());
            assert!(value
                .flatten()
                .is_none_or(|ty| tree.type_arena().unwrap().get(ty) == Type::Unknown));
        }
    };
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
}

#[test]
fn source_value_queries_retarget_namespace_provider_without_recapturing_consumer() {
    let arena = Arc::new(TypeArena::new());
    let source = "import * as ns from './barrel'; declare const alias: typeof ns.tag; export interface Uses { [alias](): void; }";
    let provider = "export declare const tag: unique symbol;";
    let files = parse(
        &arena,
        &[
            ("left.ts", provider),
            ("right.ts", provider),
            ("barrel.ts", "export { tag } from './left';"),
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
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    let check = |tree: &Compilation, path: &str| {
        let target = unique(
            &tree.program_lookup(path).unwrap(),
            provider,
            "tag: unique symbol",
        );
        assert_eq!(
            key(&tree.program_lookup("main.ts").unwrap(), source, "[alias]"),
            Some(Some(target))
        );
        let alias = ids
            .row_id(
                "main.ts",
                files[3]
                    .symbols
                    .iter()
                    .position(|s| s.name == "alias")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            tree.program_lookup("main.ts")
                .unwrap()
                .field_type_id_of(alias),
            Some(target)
        );
    };
    check(&tree, "left.ts");
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(&arena, &[("barrel.ts", "export { tag } from './right';")]);
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let updated = context(&[&files[0], &files[1], &changed[0], &files[3]]);
    let mut edited = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&updated),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, "right.ts");
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, "right.ts");
    db.conn()
        .execute("DELETE FROM files WHERE path='right.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), restored);
    deleted.ingest_from_db(db.conn());
    assert!(key(
        &deleted.program_lookup("main.ts").unwrap(),
        source,
        "[alias]"
    )
    .flatten()
    .is_none());
}

#[test]
fn source_value_queries_survive_rowless_portable_member_signatures() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let original = TypeArena::new();
    let source = "function discard() {} interface Keys { readonly token: unique symbol; } declare const keys: Keys; declare const alias: typeof keys.token; interface Uses { [alias](): void; }";
    let mut files = parse(&original, &[("provider.d.ts", source)]);
    let discarded = files[0]
        .symbols
        .iter()
        .position(|s| s.name == "discard")
        .unwrap();
    let property = files[0]
        .symbols
        .iter()
        .position(|s| s.name == "token")
        .unwrap();
    files[0].symbols[property].parent_index = Some(discarded);
    reduce_to_contract(&mut files[0]);
    assert!(!files[0].symbols.iter().any(|s| s.name == "token"));
    let payload = serde_json::to_string(&CachedParse::from_parsed(&files[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut file = cached.into_parsed(
        &arena,
        "provider.d.ts",
        &files[0].content_hash,
        files[0].size,
        None,
    );
    file.content = Some(source.into());
    reduce_to_contract(&mut file);
    assert!(!file.symbols.iter().any(|s| s.name == "token"));
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&file),
        "external",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&file])),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("provider.d.ts").unwrap();
        assert_eq!(
            key(&lookup, source, "[alias]"),
            Some(Some(unique(
                &lookup,
                source,
                "readonly token: unique symbol"
            )))
        );
    };
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
}

#[test]
fn source_value_queries_preserve_program_and_source_hash_isolation() {
    use crate::indexer::resolve::engine::contract::FlowCacheLookup;
    let arena = Arc::new(TypeArena::new());
    let source = "declare const tag: unique symbol; declare const alias: typeof tag; interface Uses { [alias](): void; }";
    let files = parse(&arena, &[("left.ts", source), ("right.ts", source)]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut config = context(&[&files[0]]);
    let mut other = context(&[&files[1]]).programs.unwrap().remove(0);
    other.key = "other".into();
    other.fingerprint = "other".into();
    config.programs.as_mut().unwrap().push(other);
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    let start = source.find("typeof tag").unwrap() as u32;
    let site = SourceSpan {
        start,
        end: start + "typeof tag".len() as u32,
    };
    let stale = parse(&arena, &[("left.ts", &format!("{source}\n"))]).remove(0);
    assert_ne!(stale.content_hash, files[0].content_hash);
    let check = |tree: &Compilation| {
        let left = tree.program_lookup("left.ts").unwrap();
        let right = tree.program_lookup("right.ts").unwrap();
        let a = left.source_value_type(site).flatten().unwrap();
        let b = right.source_value_type(site).flatten().unwrap();
        assert_ne!(a, b);
        assert!(!(&left as &dyn SymbolLookup).accepts_type_context(tree.type_arena().unwrap(), b));
        assert!(!(&right as &dyn SymbolLookup).accepts_type_context(tree.type_arena().unwrap(), a));
        assert_eq!(key(&left, source, "[alias]"), Some(Some(a)));
        assert_eq!(key(&right, source, "[alias]"), Some(Some(b)));
        assert_eq!(
            tree.source_program_lookup(&stale)
                .unwrap()
                .source_value_type(site),
            Some(None)
        );
    };
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
    let mut overlap = config.clone();
    let shared = overlap.programs.as_ref().unwrap()[0].sources[0].clone();
    overlap.programs.as_mut().unwrap()[1].sources.push(shared);
    let ambiguous =
        Compilation::build_with_context(&files, &ids, arena, Some(&overlap), &HashSet::new());
    assert_eq!(
        ambiguous
            .program_lookup("left.ts")
            .unwrap()
            .source_value_type(site),
        Some(None)
    );
}
