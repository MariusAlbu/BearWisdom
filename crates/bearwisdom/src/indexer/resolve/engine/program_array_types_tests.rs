use super::super::merge_proof::tests::{context, owner, parse};
use super::*;
use crate::indexer::{contract_filter::reduce_to_contract, external_parse_payload::CachedParse};
use std::{collections::HashSet, sync::Arc};

const PROVIDER: &str = "interface Array<T> { [n: number]: T; length: number } interface ReadonlyArray<T> { readonly [n: number]: T; readonly length: number }";

#[test]
fn compiler_array_constructor_cohort_preserves_portable_poisoned_and_cold_results() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/array_initializer_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        let source = format!("export {{}}; {}", case["source"].as_str().unwrap());
        let original = TypeArena::new();
        let providers = parse(
            &original,
            &[("arrays.d.ts", PROVIDER), ("main.ts", &source)],
        );
        let arena = Arc::new(TypeArena::new());
        for value in 0..32 {
            arena.intern(Type::Literal(
                crate::type_checker::core::types::LitValue::Int(value),
            ));
        }
        let mut files = Vec::new();
        for provider in providers {
            let payload =
                serde_json::to_string(&CachedParse::from_parsed(&provider, &original)).unwrap();
            let cached: CachedParse = serde_json::from_str(&payload).unwrap();
            let mut file = cached.into_parsed(
                &arena,
                &provider.path,
                &provider.content_hash,
                provider.size,
                None,
            );
            file.content = provider.content;
            reduce_to_contract(&mut file);
            files.push(file);
        }
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &files,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let value = owner(&ids, &files[1], "value");
        let expected = case["supported"]
            .as_bool()
            .unwrap()
            .then(|| owner(&ids, &files[1], "expected"));
        for file in &mut files {
            for symbol in &mut file.symbols {
                symbol.name = "poisoned".into();
                symbol.qualified_name = "poisoned.display".into();
                symbol.signature = None;
            }
        }
        let check = |tree: &Compilation| {
            let lookup = tree.program_lookup("main.ts").unwrap();
            let actual = lookup.field_type_id_of(value);
            if let Some(expected) = expected {
                assert_eq!(
                    actual,
                    lookup.field_type_id_of(expected),
                    "{}",
                    case["name"]
                );
                assert!(actual.is_some_and(|ty| !matches!(
                    tree.type_arena().unwrap().get(ty),
                    Type::Unknown
                )));
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
        };
        let tree = Compilation::build_with_context(
            &files,
            &ids,
            Arc::clone(&arena),
            Some(&context(&files.iter().collect::<Vec<_>>())),
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
fn missing_array_role_is_not_inferred_from_display_spelling() {
    let view = View::empty();
    let arena = std::sync::Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), std::sync::Arc::clone(&arena));
    let lookup = Lookup {
        tree: &tree,
        view: &view,
        source: None,
    };
    let element = arena.intern(Type::Intrinsic(
        crate::type_checker::core::types::Intrinsic::String,
    ));
    let base = arena.decl_in(view.context, "Array", 42);
    let ty = arena.intern(Type::Apply {
        base,
        args: vec![element],
    });
    assert!(shape(&lookup, &arena, ty).is_none());
    assert_eq!(readonly(&lookup, &arena, ty), None);
}

#[test]
fn array_roles_follow_selected_provider_changes_and_revoke_after_deletion() {
    use crate::indexer::symbol_ids::SymbolIds;
    let arena = Arc::new(TypeArena::new());
    let source = "export {}; interface Result<T> { read(): T } interface Factory { new<T>(input: readonly T[]): Result<T> } declare const Build: Factory; declare const items: string[]; declare const frozen: readonly string[]; class Holder { value = new Build(items) } declare const expected: Result<string>;";
    let files = parse(
        &arena,
        &[
            ("left.d.ts", PROVIDER),
            ("right.d.ts", PROVIDER),
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
    let field = owner(&ids, &files[2], "value");
    let expected = owner(&ids, &files[2], "expected");
    let input = owner(&ids, &files[2], "items");
    let frozen = owner(&ids, &files[2], "frozen");
    let left = owner(&ids, &files[0], "Array");
    let right = owner(&ids, &files[1], "Array");
    let check = |tree: &Compilation, wanted: Option<i64>| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        let arena = tree.type_arena().unwrap();
        let actual = lookup.field_type_id_of(field);
        if let Some(wanted) = wanted {
            assert_eq!(
                lookup.view.arrays.0.get(&Kind::Mutable),
                Some(&Some(wanted))
            );
            assert_eq!(actual, lookup.field_type_id_of(expected));
            assert!(actual.is_some_and(|ty| !matches!(arena.get(ty), Type::Unknown)));
            let proof = super::super::merge_proof::types::Relation {
                lookup: &lookup,
                arena,
            };
            let mutable = proof
                .canonical(lookup.field_type_id_of(input).unwrap(), 0)
                .unwrap();
            let frozen = proof
                .canonical(lookup.field_type_id_of(frozen).unwrap(), 0)
                .unwrap();
            assert_eq!(shape(&lookup, arena, mutable).unwrap().kind, Kind::Mutable);
            assert_eq!(shape(&lookup, arena, frozen).unwrap().kind, Kind::Readonly);
            assert_eq!(readonly(&lookup, arena, mutable), Some(frozen));
            assert!(relation(&lookup, arena, mutable, frozen).unwrap().is_ok());
            assert_eq!(relation(&lookup, arena, frozen, mutable), Some(Err(())));
            // Even the same physical row cannot be borrowed from another
            // configured nominal context, with or without plausible display.
            let base = arena.decl_in(
                crate::type_checker::core::types::NominalContextId::fresh(),
                "Array",
                wanted,
            );
            let foreign = arena.intern(Type::Apply {
                base,
                args: vec![shape(&lookup, arena, mutable).unwrap().element],
            });
            assert!(shape(&lookup, arena, foreign).is_none());
            assert!(readonly(&lookup, arena, foreign).is_none());
        } else {
            assert!(
                actual.is_none_or(|ty| matches!(arena.get(ty), Type::Unknown)),
                "unselected/stale provider fabricated inference"
            );
            assert!(lookup
                .view
                .arrays
                .0
                .get(&Kind::Readonly)
                .copied()
                .flatten()
                .is_none());
        }
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0], &files[2]])),
        &HashSet::new(),
    );
    check(&tree, Some(left));
    tree.persist_type_info(db.conn()).unwrap();
    // No file edits: only the configured provider selection changes.
    let mut switched = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&arena),
        Some(&context(&[&files[1], &files[2]])),
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
    let changed = parse(
        &restored,
        &[(
            "right.d.ts",
            "interface Array<T> { [n: number]: T; length: number }",
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
        Some(&context(&[&changed[0], &files[2]])),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, None);
    edited.persist_type_info(db.conn()).unwrap();
    db.conn()
        .execute("DELETE FROM files WHERE path='right.d.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&restored),
        Some(&context(&[&files[2]])),
        &HashSet::new(),
    );
    deleted.ingest_from_db(db.conn());
    check(&deleted, None);
    deleted.persist_type_info(db.conn()).unwrap();
    let final_arena = Arc::new(TypeArena::new());
    final_arena.restore_snapshot(&restored.serialize_snapshot());
    let mut final_cold = Compilation::build(&[], &SymbolIds::default(), final_arena);
    final_cold.ingest_from_db(db.conn());
    check(&final_cold, None);
}

#[test]
fn array_roles_require_source_interfaces_with_one_generic_parameter() {
    for provider in [
        "interface Array<T, U> { first: T; second: U } interface ReadonlyArray<T> { value: T }",
        "class Array<T> { value: T } interface ReadonlyArray<T> { value: T }",
        "interface Array<T> { value: T } class ReadonlyArray<T> { value: T }",
        "interface Array<T> { value: T } interface ReadonlyArray<T, U> { first: T; second: U }",
    ] {
        let arena = Arc::new(TypeArena::new());
        let files = parse(&arena, &[("arrays.d.ts", provider), ("main.ts", "export {}; declare const input: string[]; declare const frozen: readonly string[];")]);
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
        let ty = lookup
            .field_type_id_of(owner(&ids, &files[1], "input"))
            .unwrap();
        assert!(
            readonly(&lookup, &arena, ty).is_none(),
            "unattested array role: {provider}"
        );
    }
}
