use super::*;

pub(super) fn with_relation(check: impl FnOnce(&Relation<'_>)) {
    let arena = std::sync::Arc::new(TypeArena::new());
    let view = View::empty();
    let tree = Compilation::build(&[], &Default::default(), std::sync::Arc::clone(&arena));
    let lookup = Lookup {
        tree: &tree,
        view: &view,
        source: None,
    };
    check(&Relation {
        lookup: &lookup,
        arena: &arena,
    });
}

#[test]
fn configured_structural_assignment_matches_compiler_diagnostics() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/structural_relation_fixtures.json"
    ))
    .unwrap();
    check_configured_cases(cases, "assignable");
}

#[test]
fn configured_nominal_structural_queries_match_compiler_evidence() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/nominal_structural_fixtures.json"
    ))
    .unwrap();
    check_configured_cases(cases, "assignable");
}

#[test]
fn configured_conditional_inference_matches_compiler_evidence() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/conditional_relation_fixtures.json"
    ))
    .unwrap();
    check_configured_cases(cases, "assignable");
}

#[test]
fn configured_conditional_obligations_retain_invalid_and_unsupported_barriers() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/conditional_obligation_fixtures.json"
    ))
    .unwrap();
    check_configured_cases(cases, "proved");
}

#[test]
fn configured_alias_proofs_require_valid_constraints_and_terminate_cycles() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/structural_obligation_fixtures.json"
    ))
    .unwrap();
    check_configured_cases(cases, "proved");
}

fn check_configured_cases(cases: Vec<serde_json::Value>, expected: &str) {
    use super::super::tests::{context, owner, parse};
    use std::{collections::HashSet, sync::Arc};
    for case in cases {
        use crate::indexer::{
            contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
        };
        let original = TypeArena::new();
        let source = format!(
            "export {{}}; {} type From = {}; type To = {};",
            case["preamble"].as_str().unwrap_or(""),
            case["from"].as_str().unwrap(),
            case["to"].as_str().unwrap()
        );
        let mut providers = parse(&original, &[("main.ts", &source)]);
        reduce_to_contract(&mut providers[0]);
        let payload =
            serde_json::to_string(&CachedParse::from_parsed(&providers[0], &original)).unwrap();
        let arena = Arc::new(TypeArena::new());
        arena.class("shift portable type IDs");
        let cached: CachedParse = serde_json::from_str(&payload).unwrap();
        let mut file = cached.into_parsed(
            &arena,
            "main.ts",
            &providers[0].content_hash,
            providers[0].size,
            None,
        );
        file.content = Some(source.clone());
        reduce_to_contract(&mut file);
        let mut files = vec![file];
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &files,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let (from_id, to_id) = (owner(&ids, &files[0], "From"), owner(&ids, &files[0], "To"));
        let tree = Compilation::build_with_context(
            &files,
            &ids,
            Arc::clone(&arena),
            Some(&context(&[&files[0]])),
            &HashSet::new(),
        );
        let check = |tree: &Compilation| {
            let lookup = tree.program_lookup("main.ts").unwrap();
            let arena = tree.type_arena().unwrap();
            let relation = Relation {
                lookup: &lookup,
                arena,
            };
            let from = (&lookup as &dyn SymbolLookup)
                .declaration_type(arena, from_id)
                .unwrap();
            let to = (&lookup as &dyn SymbolLookup)
                .declaration_type(arena, to_id)
                .unwrap();
            assert_eq!(
                relation.assignable(from, to),
                case[expected].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        };
        check(&tree);
        tree.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &Default::default(), restored);
        cold.ingest_from_db(db.conn());
        check(&cold);
        for symbol in &mut files[0].symbols {
            symbol.name = "poisoned".into();
            symbol.qualified_name = "poisoned.display".into();
            symbol.signature = None;
        }
        let poisoned = Compilation::build_with_context(
            &files,
            &ids,
            Arc::clone(&arena),
            Some(&context(&[&files[0]])),
            &HashSet::new(),
        );
        check(&poisoned);
    }
}

#[test]
fn unknown_identity_does_not_prove_type_compatibility() {
    let arena = std::sync::Arc::new(TypeArena::new());
    let view = View::empty();
    let tree = Compilation::build(&[], &Default::default(), std::sync::Arc::clone(&arena));
    let lookup = Lookup {
        tree: &tree,
        view: &view,
        source: None,
    };
    let relation = Relation {
        lookup: &lookup,
        arena: &arena,
    };
    let missing = arena.intern(Type::Unknown);
    let top = arena.intern(Type::Intrinsic(Intrinsic::Unknown));
    assert!(!relation.equal(missing, missing));
    assert!(!relation.assignable(missing, top));
    assert!(relation.equal(top, top));
    assert!(relation.assignable(arena.intern(Type::Intrinsic(Intrinsic::String)), top));
    assert!(!relation.assignable(top, arena.intern(Type::Intrinsic(Intrinsic::String))));
}

#[test]
fn optional_source_distributes_over_distinct_target_union_arms() {
    let arena = std::sync::Arc::new(TypeArena::new());
    let view = View::empty();
    let tree = Compilation::build(&[], &Default::default(), std::sync::Arc::clone(&arena));
    let lookup = Lookup {
        tree: &tree,
        view: &view,
        source: None,
    };
    let relation = Relation {
        lookup: &lookup,
        arena: &arena,
    };
    let number = arena.intern(Type::Intrinsic(Intrinsic::Number));
    let undefined = arena.intern(Type::Intrinsic(Intrinsic::Undefined));
    let string = arena.intern(Type::Intrinsic(Intrinsic::String));
    let optional = arena.intern(Type::Optional(number));
    for parts in [vec![number, undefined], vec![undefined, number]] {
        let union = arena.intern(Type::Union(parts));
        assert!(relation.assignable(optional, union));
        assert!(relation.assignable(union, optional));
    }
    assert!(!relation.assignable(optional, number));
    assert!(!relation.assignable(optional, arena.intern(Type::Union(vec![string, undefined]))));
    assert!(!relation.assignable(optional, arena.intern(Type::Union(vec![string, number]))));
}

#[test]
fn evaluated_imports_and_constraints_follow_provider_retargeting_and_deletion() {
    use super::super::tests::{context, owner, parse};
    use std::{collections::HashSet, sync::Arc};
    let arena = Arc::new(TypeArena::new());
    let source = "import type { Model } from './barrel'; type Picked<T, K extends keyof T> = { [P in K]: T[P] }; type Checked<T extends { a: string }> = T; type From = Picked<Model, 'a'>; type Constrained = Checked<Model>;";
    let files = parse(
        &arena,
        &[
            ("left.ts", "export type Model = { a: string };"),
            ("right.ts", "export type Model = { a: number };"),
            ("barrel.ts", "export type { Model } from './left';"),
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
    let from = owner(&ids, &files[3], "From");
    let constrained = owner(&ids, &files[3], "Constrained");
    let check = |tree: &Compilation, value: Option<Intrinsic>| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        let arena = tree.type_arena().unwrap();
        let relation = Relation {
            lookup: &lookup,
            arena,
        };
        let from = (&lookup as &dyn SymbolLookup)
            .declaration_type(arena, from)
            .unwrap();
        let constrained = (&lookup as &dyn SymbolLookup)
            .declaration_type(arena, constrained)
            .unwrap();
        assert_eq!(
            relation.canonical(constrained, 0).is_some(),
            value == Some(Intrinsic::String)
        );
        if let Some(value) = value {
            let expected = arena.intern(Type::Operator(TypeOperator::Object(vec![TypeProperty {
                key: arena.intern(Type::Literal(LitValue::Str("a".into()))),
                value: arena.intern(Type::Intrinsic(value)),
                optional: false,
                readonly: false,
                index: false,
            }])));
            assert!(relation.equal(from, expected));
        } else {
            assert_eq!(relation.canonical(from, 0), None);
        }
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    check(&tree, Some(Intrinsic::String));
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(
        &arena,
        &[("barrel.ts", "export type { Model } from './right';")],
    );
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
    check(&edited, Some(Intrinsic::Number));
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &Default::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, Some(Intrinsic::Number));
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
fn nominal_conditional_values_and_origins_follow_provider_changes_without_consumer_recapture() {
    use super::super::tests::{context, owner, parse};
    use std::{collections::HashSet, sync::Arc};
    let arena = Arc::new(TypeArena::new());
    let main = "import type { Model } from './barrel'; type Nominal = Model; type Chosen<T> = T extends { value: infer U } ? U : bigint; type From = Chosen<Model>; type Keys = keyof Model;";
    let files = parse(
        &arena,
        &[
            ("left.ts", "export interface Model { value: string }"),
            ("right.ts", "export interface Model { value: number }"),
            ("barrel.ts", "export type { Model } from './left';"),
            ("main.ts", main),
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
    let from = owner(&ids, &files[3], "From");
    let nominal = owner(&ids, &files[3], "Nominal");
    let keys = owner(&ids, &files[3], "Keys");
    let check = |tree: &Compilation, value: Option<Intrinsic>, origin: Option<&str>| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        let arena = tree.type_arena().unwrap();
        let relation = Relation {
            lookup: &lookup,
            arena,
        };
        let evaluate = |row| {
            relation.canonical(
                (&lookup as &dyn SymbolLookup)
                    .declaration_type(arena, row)
                    .unwrap(),
                0,
            )
        };
        assert_eq!(
            evaluate(from),
            value.map(|v| arena.intern(Type::Intrinsic(v)))
        );
        if value.is_none() {
            assert_eq!(evaluate(nominal), None);
            assert_eq!(evaluate(keys), None);
            return;
        }
        let owner = crate::indexer::resolve::engine::head_decl::head_decl_id(
            arena,
            evaluate(nominal).unwrap(),
        )
        .unwrap();
        let members = &lookup.nominal_surface(owner).unwrap().members;
        if let Some(path) = origin {
            assert_eq!(members.len(), 1);
            let origin = &members[0].origin;
            assert_eq!(
                lookup
                    .symbol_by_id(origin.declaration.unwrap())
                    .unwrap()
                    .file_path
                    .as_ref(),
                path
            );
            assert!(lookup.view.sources[&origin.source]
                .signatures
                .contains_key(&origin.signature));
            assert_eq!(evaluate(keys), Some(members[0].property.key));
        } else {
            assert!(members.is_empty());
            assert_eq!(
                evaluate(keys),
                Some(arena.intern(Type::Intrinsic(Intrinsic::Never)))
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
    check(&tree, Some(Intrinsic::String), Some("left.ts"));
    tree.persist_type_info(db.conn()).unwrap();
    let barrel = parse(
        &arena,
        &[("barrel.ts", "export type { Model } from './right';")],
    )
    .remove(0);
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&barrel),
        "internal",
        Some(&arena),
    )
    .unwrap();
    let config = context(&[&files[0], &files[1], &barrel, &files[3]]);
    let mut edited = Compilation::build_with_context(
        std::slice::from_ref(&barrel),
        &changed_ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, Some(Intrinsic::Number), Some("right.ts"));
    edited.persist_type_info(db.conn()).unwrap();
    let empty = parse(&arena, &[("right.ts", "export interface Model {}")]).remove(0);
    let (_, empty_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&empty),
        "internal",
        Some(&arena),
    )
    .unwrap();
    let config = context(&[&files[0], &empty, &barrel, &files[3]]);
    let mut edited = Compilation::build_with_context(
        std::slice::from_ref(&empty),
        &empty_ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, Some(Intrinsic::BigInt), None);
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &Default::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, Some(Intrinsic::BigInt), None);
    db.conn()
        .execute("DELETE FROM files WHERE path='right.ts'", [])
        .unwrap();
    let config = context(&[&files[0], &barrel, &files[3]]);
    let mut deleted = Compilation::build_with_context(
        &[],
        &Default::default(),
        restored,
        Some(&config),
        &HashSet::new(),
    );
    deleted.ingest_from_db(db.conn());
    check(&deleted, None, None);
}
