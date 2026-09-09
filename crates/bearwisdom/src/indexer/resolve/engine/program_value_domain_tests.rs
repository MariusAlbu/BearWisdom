use super::*;

#[test]
fn erased_imports_retarget_and_delete_without_runtime_or_global_fallback() {
    let arena = Arc::new(TypeArena::new());
    let source = "import type { tag } from './barrel'; interface Uses { [tag](): void; } class Runtime { [tag](): void {} }";
    let provider = "export declare const tag: unique symbol;";
    let files = parse(
        &arena,
        &[
            ("left.ts", provider),
            ("right.ts", provider),
            ("barrel.ts", "export type { tag } from './left';"),
            ("main.ts", source),
            ("global.d.ts", "declare const tag: unique symbol;"),
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
    let check = |tree: &Compilation, expected: &str| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        let wanted = unique(
            &tree.program_lookup(expected).unwrap(),
            provider,
            "tag: unique symbol",
        );
        assert_eq!(key(&lookup, source, "[tag]"), Some(Some(wanted)));
        let runtime = source.rfind("[tag]").unwrap() as u32;
        assert_eq!(
            lookup.computed_key(SourceSpan {
                start: runtime,
                end: runtime + 5
            }),
            Some(None)
        );
        for other in ["left.ts", "right.ts", "global.d.ts"]
            .into_iter()
            .filter(|path| *path != expected)
        {
            let text = files
                .iter()
                .find(|file| file.path == other)
                .unwrap()
                .content
                .as_deref()
                .unwrap();
            assert_ne!(
                wanted,
                unique(
                    &tree.program_lookup(other).unwrap(),
                    text,
                    "tag: unique symbol"
                )
            );
        }
    };
    check(&tree, "left.ts");
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(
        &arena,
        &[("barrel.ts", "export type { tag } from './right';")],
    );
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let config = context(&[&files[0], &files[1], &changed[0], &files[3], &files[4]]);
    let mut edited = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&config),
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
    assert!(
        key(&deleted.program_lookup("main.ts").unwrap(), source, "[tag]")
            .flatten()
            .is_none()
    );
}

#[test]
fn pure_types_preserve_program_value_identity_through_portable_cold_and_overlap() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let original = TypeArena::new();
    let provider = "interface Factory { readonly tag: unique symbol; } declare var Token: Factory;";
    let consumer = "export {}; interface Token {} declare const alias: typeof Token.tag; interface Uses { [alias](): void; }";
    let sources = [
        ("left.d.ts", provider),
        ("right.d.ts", provider),
        ("a.ts", consumer),
        ("b.ts", consumer),
    ];
    let arena = Arc::new(TypeArena::new());
    let files: Vec<_> = parse(&original, &sources)
        .into_iter()
        .map(|mut file| {
            reduce_to_contract(&mut file);
            let payload =
                serde_json::to_string(&CachedParse::from_parsed(&file, &original)).unwrap();
            let cache: CachedParse = serde_json::from_str(&payload).unwrap();
            let mut restored =
                cache.into_parsed(&arena, &file.path, &file.content_hash, file.size, None);
            restored.content = Some(
                sources
                    .iter()
                    .find(|(path, _)| *path == file.path)
                    .unwrap()
                    .1
                    .into(),
            );
            reduce_to_contract(&mut restored);
            restored
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
    let mut config = context(&[&files[0], &files[2]]);
    let mut second = context(&[&files[1], &files[3]]).programs.unwrap().remove(0);
    second.key = "second".into();
    second.fingerprint = "second".into();
    config.programs.as_mut().unwrap().push(second);
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let left = unique(
            &tree.program_lookup("left.d.ts").unwrap(),
            provider,
            "readonly tag: unique symbol",
        );
        let right = unique(
            &tree.program_lookup("right.d.ts").unwrap(),
            provider,
            "readonly tag: unique symbol",
        );
        assert_ne!(left, right);
        assert_eq!(
            key(&tree.program_lookup("a.ts").unwrap(), consumer, "[alias]"),
            Some(Some(left))
        );
        assert_eq!(
            key(&tree.program_lookup("b.ts").unwrap(), consumer, "[alias]"),
            Some(Some(right))
        );
    };
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
    let shared = config.programs.as_ref().unwrap()[0].sources[1].clone();
    config.programs.as_mut().unwrap()[1].sources.push(shared);
    let ambiguous =
        Compilation::build_with_context(&files, &ids, arena, Some(&config), &HashSet::new());
    assert_eq!(
        key(
            &ambiguous.program_lookup("a.ts").unwrap(),
            consumer,
            "[alias]"
        ),
        Some(None)
    );
    assert!(key(
        &ambiguous.program_lookup("b.ts").unwrap(),
        consumer,
        "[alias]"
    )
    .flatten()
    .is_some());
}

#[test]
fn erased_namespace_queries_keep_ambiguous_and_cyclic_providers_unknown() {
    let arena = Arc::new(TypeArena::new());
    let provider = "export declare const tag: unique symbol;";
    let source = "import type * as ns from './barrel'; declare const alias: typeof ns.tag; interface Uses { [alias](): void; }";
    for barrel in [
        "export type * from './left'; export type * from './right';",
        "export type * from './cycle';",
    ] {
        let files = parse(
            &arena,
            &[
                ("left.ts", provider),
                ("right.ts", provider),
                ("barrel.ts", barrel),
                ("cycle.ts", "export type * from './barrel';"),
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
        assert_eq!(
            key(&tree.program_lookup("main.ts").unwrap(), source, "[alias]"),
            Some(None)
        );
        tree.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
        cold.ingest_from_db(db.conn());
        assert_eq!(
            key(&cold.program_lookup("main.ts").unwrap(), source, "[alias]"),
            Some(None)
        );
    }
}
