use super::merge_proof::tests::{context, owner, parse};
use super::*;
use crate::indexer::resolve::engine::{
    contract::{FileContext, FlowCacheLookup},
    file_lookup::FileLookup,
    semantic_model::{SemanticModel, SolveOutcome},
    testkit,
};
use crate::indexer::{
    external_parse_payload::CachedParse, programs::CallablePolicy, symbol_ids::SymbolIds,
};
use crate::type_checker::core::types::{Intrinsic, Type, TypeId};
use std::{collections::HashSet, sync::Arc};

#[path = "program_conditional_receiver_state_tests.rs"]
mod state;

fn policy(files: &[&crate::types::ParsedFile]) -> crate::indexer::resolve::ProjectContext {
    let mut config = context(files);
    config.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
        strict_parameters: true,
        strict_nulls: true,
        bivariant_methods: Some(true),
    });
    config
}

fn outcome(
    tree: &Compilation,
    file: &crate::types::ParsedFile,
    ids: &SymbolIds,
    selector: u32,
) -> SolveOutcome {
    let lookup = FileLookup::for_file(tree, file, ids);
    let reference = file
        .refs
        .iter()
        .find(|r| {
            r.kind == crate::types::EdgeKind::Calls
                && r.chain
                    .as_ref()
                    .is_some_and(|c| c.segments.last().unwrap().byte_offset == selector)
        })
        .unwrap();
    let mut site = testkit::ref_ctx(
        reference,
        &file.symbols[reference.source_symbol_index],
        vec![],
    );
    site.source_symbol_id = ids.row_id(&file.path, reference.source_symbol_index);
    lookup.set_cursor(reference.byte_offset);
    let ctx = FileContext {
        file_path: file.path.clone(),
        language: "typescript".into(),
        imports: vec![],
        file_namespace: None,
    };
    SemanticModel::production().get_symbol_info(
        &site,
        &ctx,
        &lookup,
        &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
    )
}

fn parts(arena: &TypeArena, ty: TypeId) -> Vec<TypeId> {
    let mut result = match arena.get(ty) {
        Type::Union(parts) => parts,
        _ => vec![ty],
    };
    result.sort_unstable();
    result
}

#[test]
fn conditional_receiver_compiler_cases_keep_exact_origins_and_unknown_barriers() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/conditional_receiver_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        for prefix in ["", "export {}; "] {
            let source = format!("{prefix}{}", case["source"].as_str().unwrap());
            let original = TypeArena::new();
            let parsed = parse(&original, &[("main.ts", &source)]).remove(0);
            let payload =
                serde_json::to_string(&CachedParse::from_parsed(&parsed, &original)).unwrap();
            let arena = Arc::new(TypeArena::new());
            for n in 0..32 {
                arena.intern(Type::Literal(
                    crate::type_checker::core::types::LitValue::Int(n),
                ));
            }
            let cached: CachedParse = serde_json::from_str(&payload).unwrap();
            let mut file =
                cached.into_parsed(&arena, "main.ts", &parsed.content_hash, parsed.size, None);
            file.content = Some(source.clone());
            crate::indexer::contract_bindings::restore(&mut file);
            let db = crate::Database::open_in_memory().unwrap();
            let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
                &db,
                std::slice::from_ref(&file),
                "internal",
                Some(&arena),
            )
            .unwrap();
            let row = |text: &str| {
                let byte = source.find(text).unwrap();
                let slot = file
                    .symbols
                    .iter()
                    .position(|s| s.start_line == 0 && s.start_col as usize == byte)
                    .unwrap();
                ids.row_id("main.ts", slot).unwrap()
            };
            let payload_row = row("interface Payload");
            let other_row = row("interface Other");
            let labels: Vec<_> = case["calls"]
                .as_array()
                .unwrap()
                .iter()
                .map(|label| {
                    let expression = label["expression"].as_str().unwrap();
                    let selector =
                        source.find(expression).unwrap() + expression.rfind('.').unwrap() + 1;
                    let target = label["declaration"]
                        .as_str()
                        .map(|text| (row(text), source.find(text).unwrap() as u32));
                    (label, selector as u32, target)
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
            let check = |tree: &Compilation| {
                let arena = tree.type_arena().unwrap();
                let program = tree.program_lookup("main.ts").unwrap();
                for (label, selector, target) in &labels {
                    let actual = outcome(tree, &file, &ids, *selector);
                    if case["supported"] == false {
                        assert!(
                            !matches!(actual, SolveOutcome::Resolved(_)),
                            "{} ({prefix}): unsupported receiver reached {selector}",
                            case["name"]
                        );
                        continue;
                    }
                    let SolveOutcome::Resolved(info) = actual else {
                        panic!("{} ({prefix}): stopped at {selector}", case["name"]);
                    };
                    let (target, span) = target.unwrap();
                    assert_eq!(
                        info.target_symbol_id, target,
                        "{}: declaration",
                        case["name"]
                    );
                    let origins: Vec<_> = program
                        .view
                        .nominal_surfaces
                        .values()
                        .flat_map(|s| &s.members)
                        .filter(|m| {
                            m.origin.declaration == Some(target)
                                && m.origin.signature.0.start == span
                        })
                        .collect();
                    assert!(
                        !origins.is_empty(),
                        "{}: missing exact source signature",
                        case["name"]
                    );
                    assert!(origins
                        .iter()
                        .all(|m| Some(m.origin.source) == program.source.unwrap().identity));
                    let actual = program
                        .evaluated_receiver(info.resolved_yield_type.unwrap())
                        .unwrap()
                        .unwrap();
                    let mut expected: Vec<_> = label["result"]
                        .as_str()
                        .unwrap()
                        .split(" | ")
                        .map(|text| match text {
                            "number" => arena.intern(Type::Intrinsic(Intrinsic::Number)),
                            "string" => arena.intern(Type::Intrinsic(Intrinsic::String)),
                            "undefined" => arena.intern(Type::Intrinsic(Intrinsic::Undefined)),
                            "Payload" => (&program as &dyn SymbolLookup)
                                .declaration_type(arena, payload_row)
                                .unwrap(),
                            "Other" => (&program as &dyn SymbolLookup)
                                .declaration_type(arena, other_row)
                                .unwrap(),
                            _ => panic!("unhandled compiler result {text}"),
                        })
                        .collect();
                    expected.sort_unstable();
                    assert_eq!(parts(arena, actual), expected, "{}: result", case["name"]);
                }
            };
            let tree = Compilation::build_with_context(
                std::slice::from_ref(&file),
                &ids,
                Arc::clone(&arena),
                Some(&policy(&[&file])),
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
}
