use super::*;
use crate::indexer::resolve::engine::contract::FlowCacheLookup;
use crate::types::SourceSpan;

fn key(tree: &Compilation, path: &str, source: &str, expression: &str) -> Option<TypeId> {
    let start = source.find(expression).unwrap() as u32;
    tree.program_lookup(path)
        .unwrap()
        .computed_key(SourceSpan {
            start,
            end: start + expression.len() as u32,
        })
        .flatten()
}

fn token(tree: &Compilation, path: &str, source: &str) -> TypeId {
    let start = source.find("token: unique symbol").unwrap() as u32;
    tree.program_lookup(path)
        .unwrap()
        .source_unique_symbol(SourceSpan {
            start,
            end: start + 20,
        })
        .unwrap()
}

#[test]
fn compiler_labelled_query_proof_cascades_and_negatives_fresh_and_cold() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/query_proof_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        let arena = Arc::new(TypeArena::new());
        let source = case["source"].as_str().unwrap();
        let files = parse(&arena, &[("main.ts", source), ("healthy.d.ts", HEALTHY)]);
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &files,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let uses = owner(&ids, &files[0], "Uses");
        let healthy = owner(&ids, &files[1], "Healthy");
        let tree = Compilation::build_with_context(
            &files,
            &ids,
            Arc::clone(&arena),
            Some(&context(&files.iter().collect::<Vec<_>>())),
            &HashSet::new(),
        );
        let check = |tree: &Compilation| {
            let lookup = tree.program_lookup("main.ts").unwrap();
            assert!(
                lookup.symbol_by_id(healthy).is_some(),
                "{}: unrelated merge",
                case["name"]
            );
            let admitted = case["admitted"].as_bool().unwrap();
            assert_eq!(
                lookup.symbol_by_id(uses).is_some(),
                admitted,
                "{}: dependent merge admission",
                case["name"]
            );
            let result = key(tree, "main.ts", source, case["key"].as_str().unwrap());
            assert_eq!(
                result,
                admitted.then(|| token(tree, "main.ts", source)),
                "{}: exact unique origin",
                case["name"]
            );
            let expression = case["key"].as_str().unwrap();
            for (start, _) in source.match_indices(expression) {
                assert_eq!(
                    lookup
                        .computed_key(SourceSpan {
                            start: start as u32,
                            end: (start + expression.len()) as u32
                        })
                        .flatten(),
                    result
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
fn repeated_query_provider_edits_revoke_proofs_and_deletion_readmits_without_consumer_recapture() {
    let arena = Arc::new(TypeArena::new());
    let provider = "declare const token: unique symbol; interface Keys { tag: typeof token; } declare const keys: Keys;";
    let consumer = "interface Next { tag: typeof keys.tag; } interface Next { tag: typeof keys.tag; } declare const next: Next; interface Uses { [next.tag](): void; } interface Uses { keep(): void; }";
    let files = parse(
        &arena,
        &[
            ("keys.d.ts", provider),
            ("extra.d.ts", "interface Keys { tag: typeof token; }"),
            ("main.ts", consumer),
            ("healthy.d.ts", HEALTHY),
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
    let uses = owner(&ids, &files[2], "Uses");
    let healthy = owner(&ids, &files[3], "Healthy");
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    let check = |tree: &Compilation, admitted: bool| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        assert!(lookup.symbol_by_id(healthy).is_some());
        assert_eq!(lookup.symbol_by_id(uses).is_some(), admitted);
        assert_eq!(
            key(tree, "main.ts", consumer, "[next.tag]"),
            admitted.then(|| token(tree, "keys.d.ts", provider))
        );
    };
    check(&tree, true);
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(&arena, &[("extra.d.ts", "interface Keys { tag: number; }")]);
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let updated = context(&[&files[0], &changed[0], &files[2], &files[3]]);
    let mut edited = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&updated),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, false);
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, false);
    db.conn()
        .execute("DELETE FROM files WHERE path='extra.d.ts'", [])
        .unwrap();
    let remaining = context(&[&files[0], &files[2], &files[3]]);
    let mut deleted = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&restored),
        Some(&remaining),
        &HashSet::new(),
    );
    deleted.ingest_from_db(db.conn());
    check(&deleted, true);
    deleted.persist_type_info(db.conn()).unwrap();
    let final_arena = Arc::new(TypeArena::new());
    final_arena.restore_snapshot(&restored.serialize_snapshot());
    let mut final_cold = Compilation::build(&[], &SymbolIds::default(), final_arena);
    final_cold.ingest_from_db(db.conn());
    check(&final_cold, true);
}

#[test]
fn rowless_repeated_query_signatures_survive_portable_and_poisoned_providers() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let original = TypeArena::new();
    let source = "function discard() {} declare const token: unique symbol; interface Keys { tag: typeof token; } interface Keys { tag: typeof token; } declare const keys: Keys; interface Uses { [keys.tag](): void; } interface Uses { keep(): void; }";
    let mut files = parse(&original, &[("main.ts", source)]);
    let discarded = files[0]
        .symbols
        .iter()
        .position(|s| s.name == "discard")
        .unwrap();
    for property in files[0].symbols.iter_mut().filter(|s| s.name == "tag") {
        property.parent_index = Some(discarded);
    }
    reduce_to_contract(&mut files[0]);
    assert!(!files[0].symbols.iter().any(|s| s.name == "tag"));
    let payload = serde_json::to_string(&CachedParse::from_parsed(&files[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    for value in 0..40 {
        arena.intern(Type::Literal(
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
    let tree = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&file])),
        &HashSet::new(),
    );
    assert_eq!(
        key(&tree, "main.ts", source, "[keys.tag]"),
        Some(token(&tree, "main.ts", source))
    );
    let before = serde_json::to_value(crate::indexer::resolve::engine::program_types::capture(
        &file, &ids, &tree, &arena,
    ))
    .unwrap();
    for symbol in &mut file.symbols {
        symbol.name = "poison".into();
        symbol.qualified_name = "poison.display".into();
        symbol.signature = None;
    }
    assert_eq!(
        serde_json::to_value(crate::indexer::resolve::engine::program_types::capture(
            &file, &ids, &tree, &arena
        ))
        .unwrap(),
        before
    );
    let poisoned = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&file])),
        &HashSet::new(),
    );
    assert_eq!(
        key(&poisoned, "main.ts", source, "[keys.tag]"),
        Some(token(&poisoned, "main.ts", source))
    );
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    assert_eq!(
        key(&cold, "main.ts", source, "[keys.tag]"),
        Some(token(&cold, "main.ts", source))
    );
}

#[test]
fn staging_exhaustion_does_not_claim_stability_or_reallocate_generic_owners_on_retry() {
    let arena = Arc::new(TypeArena::new());
    let source = "declare const token: unique symbol; interface Box<T> { value: T; } interface Keys { tag: typeof token; } interface Keys { tag: typeof token; } declare const keys: Keys; interface Next { tag: typeof keys.tag; } interface Next { tag: typeof keys.tag; } declare const next: Next; interface Uses { [next.tag](): void; } interface Uses { keep(): void; }";
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
    let modules = tree._test_program_modules();
    let program = modules.programs.program("only").unwrap();
    let sources = modules.programs.sources(program);
    let allowed = modules
        .programs
        .pending(program)
        .iter()
        .map(|p| p.key)
        .collect();
    let staged = modules.programs.staged(program, &allowed);
    let mut view = View::build(
        program,
        &sources,
        modules,
        &tree,
        &arena,
        &staged,
        &Default::default(),
    );
    let parameters: FxHashMap<_, _> = view
        .info
        .iter()
        .map(|(&row, info)| (row, info.generic_param_ids.clone()))
        .collect();
    assert!(parameters.values().any(|parameters| !parameters.is_empty()));
    assert!(
        settle(&mut view, program, &sources, modules, &tree, &arena, &allowed, 1).is_none(),
        "partial rounds cannot be published"
    );
    let (invalid, heritage, _) = settle(
        &mut view, program, &sources, modules, &tree, &arena, &allowed, 128,
    )
    .unwrap();
    assert!(invalid.is_empty() && heritage.is_empty());
    for (row, parameters) in parameters {
        assert_eq!(view.info[&row].generic_param_ids, parameters);
    }
}

#[test]
fn proved_query_values_follow_barrel_retargeting_and_never_borrow_deleted_namesakes() {
    let arena = Arc::new(TypeArena::new());
    let provider = "export declare const token: unique symbol;";
    let source = "import { token } from './barrel'; interface Keys { tag: typeof token; } interface Keys { tag: typeof token; } declare const keys: Keys; interface Uses { [keys.tag](): void; } interface Uses { keep(): void; }";
    let files = parse(
        &arena,
        &[
            ("left.ts", provider),
            ("right.ts", provider),
            ("barrel.ts", "export { token } from './left';"),
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
    let check = |tree: &Compilation, wanted: &str, other: &str| {
        let expected = token(tree, wanted, provider);
        let unexpected = token(tree, other, provider);
        assert_ne!(expected, unexpected);
        assert_eq!(key(tree, "main.ts", source, "[keys.tag]"), Some(expected));
    };
    check(&tree, "left.ts", "right.ts");
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(&arena, &[("barrel.ts", "export { token } from './right';")]);
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
    let mut stale = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    stale.ingest_from_db(db.conn());
    assert!(key(&stale, "main.ts", source, "[keys.tag]").is_none());
    let remaining = context(&[&files[0], &changed[0], &files[3]]);
    let mut deleted = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        restored,
        Some(&remaining),
        &HashSet::new(),
    );
    deleted.ingest_from_db(db.conn());
    assert!(key(&deleted, "main.ts", source, "[keys.tag]").is_none());
}

#[test]
fn repeated_query_proofs_are_isolated_between_overlapping_programs() {
    let arena = Arc::new(TypeArena::new());
    let provider = "declare const token: unique symbol; interface Keys { tag: typeof token; } declare const keys: Keys;";
    let consumer = "interface Uses { [keys.tag](): void; } interface Uses { keep(): void; }";
    let files = parse(
        &arena,
        &[
            ("keys.d.ts", provider),
            ("valid.d.ts", "interface Keys { tag: typeof token; }"),
            ("invalid.d.ts", "interface Keys { tag: number; }"),
            ("left.ts", consumer),
            ("right.ts", consumer),
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
    let mut config = context(&[&files[0], &files[1], &files[3]]);
    let mut other = context(&[&files[0], &files[2], &files[4]])
        .programs
        .unwrap()
        .remove(0);
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
    let check = |tree: &Compilation| {
        let left = tree.program_lookup("left.ts").unwrap();
        let right = tree.program_lookup("right.ts").unwrap();
        let selected = key(tree, "left.ts", consumer, "[keys.tag]").unwrap();
        assert!(
            (&left as &dyn SymbolLookup).accepts_type_context(tree.type_arena().unwrap(), selected)
        );
        assert!(!(&right as &dyn SymbolLookup)
            .accepts_type_context(tree.type_arena().unwrap(), selected));
        assert!(key(tree, "right.ts", consumer, "[keys.tag]").is_none());
        assert!(tree
            .program_lookup("keys.d.ts")
            .unwrap()
            .symbol_by_id(owner(&ids, &files[0], "Keys"))
            .is_none());
    };
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
}
