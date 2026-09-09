use super::*;
use crate::indexer::resolve::engine::program_view::merge_proof::tests::{context, owner, parse};
use std::{collections::HashSet, sync::Arc};

#[test]
fn callable_arguments_match_compiler_assignment_evidence() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/callable_relation_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        use crate::indexer::{
            contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
        };
        let preamble = case["preamble"].as_str().unwrap_or("");
        let array_provider = case["arrayProvider"] == true;
        let source = format!(
            "export {{}}; {} type From = {}; type To = {};",
            if array_provider { "" } else { preamble },
            case["from"].as_str().unwrap(),
            case["to"].as_str().unwrap()
        );
        let sources = if array_provider {
            vec![("arrays.d.ts", preamble), ("main.ts", source.as_str())]
        } else {
            vec![("main.ts", source.as_str())]
        };
        let original = TypeArena::new();
        let mut providers = parse(&original, &sources);
        for provider in &mut providers {
            reduce_to_contract(provider);
        }
        let payloads: Vec<_> = providers
            .iter()
            .map(|provider| {
                serde_json::to_string(&CachedParse::from_parsed(provider, &original)).unwrap()
            })
            .collect();
        let arena = Arc::new(TypeArena::new());
        for i in 0..32 {
            arena.intern(Type::Literal(LitValue::Int(i)));
        }
        let mut files = Vec::new();
        for ((provider, payload), (_, content)) in providers.iter().zip(&payloads).zip(&sources) {
            let cached: CachedParse = serde_json::from_str(payload).unwrap();
            let mut file = cached.into_parsed(
                &arena,
                &provider.path,
                &provider.content_hash,
                provider.size,
                None,
            );
            file.content = Some((*content).to_owned());
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
        let main = files
            .iter()
            .position(|file| file.path == "main.ts")
            .unwrap();
        let (from, to) = (
            owner(&ids, &files[main], "From"),
            owner(&ids, &files[main], "To"),
        );
        for file in &mut files {
            for s in &mut file.symbols {
                s.name = "poison".into();
                s.qualified_name = "poison.display".into();
                s.signature = None;
            }
        }
        let inputs: Vec<_> = files.iter().collect();
        let mut config = context(&inputs);
        config.programs.as_mut().unwrap()[0].callable_policy =
            Some(crate::indexer::programs::CallablePolicy {
                strict_parameters: case["strictParameters"].as_bool().unwrap_or(true),
                strict_nulls: case["strictNulls"].as_bool().unwrap_or(true),
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
            let lookup = tree.program_lookup("main.ts").unwrap();
            let arena = tree.type_arena().unwrap();
            let relation = Relation {
                lookup: &lookup,
                arena,
            };
            let a = (&lookup as &dyn SymbolLookup)
                .declaration_type(arena, from)
                .unwrap();
            let b = (&lookup as &dyn SymbolLookup)
                .declaration_type(arena, to)
                .unwrap();
            let expected =
                (case["supported"] != false).then(|| case["assignable"].as_bool().unwrap());
            let canonical_a = relation.canonical(a, 0);
            let canonical_b = relation.canonical(b, 0);
            let (Some(a), Some(b)) = (canonical_a, canonical_b) else {
                assert!(expected.is_none(), "{}: incomplete source type: from={} ({canonical_a:?}), to={} ({canonical_b:?})",
                    case["name"], arena.format_type(a), arena.format_type(b));
                return;
            };
            let (Type::Callable(left), Type::Callable(right)) = (arena.get(a), arena.get(b)) else {
                panic!("{}: missing source callable", case["name"]);
            };
            assert_ne!(
                left.origin, right.origin,
                "assignment must not collapse source identities"
            );
            assert_eq!(relation.argument(a, b), expected, "{}", case["name"]);
            if case["sourceDiagnostic"] == true {
                assert_eq!(
                    relation.argument(a, arena.intern(Type::Intrinsic(Intrinsic::Unknown))),
                    None,
                    "{}: broad targets cannot revive invalid callable evidence",
                    case["name"]
                );
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
fn callable_policy_changes_rebuild_proofs_and_old_sources_cannot_cross_snapshots() {
    use crate::indexer::programs::CallablePolicy;
    let arena = Arc::new(TypeArena::new());
    let source =
        "export {}; type From = (x: string) => string; type To = (x: string | number) => string;";
    let files = parse(&arena, &[("main.ts", source)]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let (from, to) = (owner(&ids, &files[0], "From"), owner(&ids, &files[0], "To"));
    let mut prior = None;
    for (policy, expected) in [
        (Some(true), Some(false)),
        (Some(false), Some(true)),
        (None, None),
    ] {
        let mut config = context(&[&files[0]]);
        config.programs.as_mut().unwrap()[0].callable_policy =
            policy.map(|strict_parameters| CallablePolicy {
                strict_parameters,
                strict_nulls: true,
                bivariant_methods: None,
            });
        let mut tree = if prior.is_none() {
            Compilation::build_with_context(
                &files,
                &ids,
                Arc::clone(&arena),
                Some(&config),
                &HashSet::new(),
            )
        } else {
            Compilation::build_with_context(
                &[],
                &Default::default(),
                Arc::clone(&arena),
                Some(&config),
                &HashSet::new(),
            )
        };
        tree.ingest_from_db(db.conn());
        let check = |tree: &Compilation, old: Option<(TypeId, TypeId)>| {
            let lookup = tree.program_lookup("main.ts").unwrap();
            let arena = tree.type_arena().unwrap();
            let relation = Relation {
                lookup: &lookup,
                arena,
            };
            if let Some((a, b)) = old {
                assert_eq!(
                    relation.argument(a, b),
                    None,
                    "stale context cannot supply new-policy evidence"
                );
            }
            let a = (&lookup as &dyn SymbolLookup)
                .declaration_type(arena, from)
                .unwrap();
            let b = (&lookup as &dyn SymbolLookup)
                .declaration_type(arena, to)
                .unwrap();
            assert_eq!(relation.argument(a, b), expected);
            (
                relation.canonical(a, 0).unwrap(),
                relation.canonical(b, 0).unwrap(),
            )
        };
        prior = Some(check(&tree, prior));
        tree.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &Default::default(), restored);
        cold.ingest_from_db(db.conn());
        check(&cold, None);
    }
}

#[test]
fn incomplete_erased_and_deep_callable_evidence_remains_unknown() {
    use crate::indexer::programs::CallablePolicy;
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[(
            "main.ts",
            "export {}; type Signature = (value: string) => number;",
        )],
    );
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let row = owner(&ids, &files[0], "Signature");
    let mut config = context(&[&files[0]]);
    config.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
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
    let lookup = tree.program_lookup("main.ts").unwrap();
    let relation = Relation {
        lookup: &lookup,
        arena: &arena,
    };
    let ty = relation
        .canonical(
            (&lookup as &dyn SymbolLookup)
                .declaration_type(&arena, row)
                .unwrap(),
            0,
        )
        .unwrap();
    let Type::Callable(c) = arena.get(ty) else {
        panic!();
    };
    let mut incomplete = c.clone();
    incomplete.complete = false;
    let incomplete = arena.intern(Type::Callable(incomplete));
    assert_eq!(relation.argument(incomplete, ty), None);
    let erased = arena.intern(Type::Function {
        params: c.parameters.iter().map(|p| p.ty).collect(),
        return_: c.result,
    });
    assert_eq!(relation.argument(erased, ty), None);
    assert_eq!(relation.argument(ty, erased), None);
    assert_eq!(relation.argument(erased, erased), None);
    let mut deep = ty;
    for _ in 0..80 {
        let mut nested = c.clone();
        nested.result = deep;
        deep = arena.intern(Type::Callable(nested));
    }
    assert_eq!(
        relation.argument(deep, deep),
        None,
        "same ID does not bypass completeness/budget checks"
    );
    assert_eq!(
        relation.argument(ty, ty),
        Some(true),
        "failed private proof must not contaminate a later query"
    );
}
