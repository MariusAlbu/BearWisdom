use super::super::tests::{context, owner, parse};
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
use std::{collections::HashSet, sync::Arc};

#[path = "program_constructor_heritage_state_tests.rs"]
mod state;

fn policy(
    files: &[&crate::types::ParsedFile],
    strict: bool,
) -> crate::indexer::resolve::ProjectContext {
    let mut config = context(files);
    config.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
        strict_parameters: strict,
        strict_nulls: strict,
        bivariant_methods: Some(true),
    });
    config
}

fn call(
    tree: &Compilation,
    file: &crate::types::ParsedFile,
    ids: &SymbolIds,
    selector: u32,
) -> Option<(i64, TypeId)> {
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
    match SemanticModel::production().get_symbol_info(
        &site,
        &ctx,
        &lookup,
        &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
    ) {
        SolveOutcome::Resolved(info) => Some((info.target_symbol_id, info.resolved_yield_type?)),
        _ => None,
    }
}

#[test]
fn constructor_heritage_compiler_cases_keep_additive_origins_and_structural_variance() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/constructor_heritage_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        for prefix in ["", "export {}; "] {
            for rowless in [false, true] {
                let source = format!("{prefix}{}", case["source"].as_str().unwrap());
                let sources = case["arrayProvider"]
                    .as_str()
                    .map(|provider| vec![("arrays.d.ts", provider), ("main.ts", source.as_str())])
                    .unwrap_or_else(|| vec![("main.ts", source.as_str())]);
                let original = TypeArena::new();
                let parsed = parse(&original, &sources);
                let payloads: Vec<_> = parsed
                    .iter()
                    .map(|file| {
                        serde_json::to_string(&CachedParse::from_parsed(file, &original)).unwrap()
                    })
                    .collect();
                let arena = Arc::new(TypeArena::new());
                for n in 0..32 {
                    arena.intern(Type::Literal(
                        crate::type_checker::core::types::LitValue::Int(n),
                    ));
                }
                let mut files = Vec::new();
                for ((parsed, payload), (_, content)) in parsed.iter().zip(&payloads).zip(&sources)
                {
                    let cached: CachedParse = serde_json::from_str(payload).unwrap();
                    let mut file = cached.into_parsed(
                        &arena,
                        &parsed.path,
                        &parsed.content_hash,
                        parsed.size,
                        None,
                    );
                    file.content = Some((*content).to_owned());
                    crate::indexer::contract_bindings::restore(&mut file);
                    files.push(file);
                }
                let main = files
                    .iter()
                    .position(|file| file.path == "main.ts")
                    .unwrap();
                if rowless {
                    for part in &mut files[main]
                        .flow
                        .lexical
                        .as_mut()
                        .unwrap()
                        .globals
                        .as_mut()
                        .unwrap()
                        .interfaces
                    {
                        for member in part
                            .surface
                            .iter_mut()
                            .flatten()
                            .filter(|m| m.kind == Kind::Construct)
                        {
                            member.slot = None;
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
                let rows: FxHashMap<_, _> = files[main]
                    .symbols
                    .iter()
                    .enumerate()
                    .map(|(slot, s)| (s.name.clone(), ids.row_id("main.ts", slot).unwrap()))
                    .collect();
                let origin_rows: FxHashMap<_, _> = files[main]
                    .symbols
                    .iter()
                    .enumerate()
                    .map(|(slot, s)| (s.start_col, ids.row_id("main.ts", slot).unwrap()))
                    .collect();
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
                let inputs: Vec<_> = files.iter().collect();
                let config = policy(&inputs, case["strict"].as_bool().unwrap_or(true));
                let check = |tree: &Compilation| {
                    let arena = tree.type_arena().unwrap();
                    let lookup = tree.program_lookup("main.ts").unwrap();
                    let relation = types::Relation {
                        lookup: &lookup,
                        arena,
                    };
                    if case["mode"] == "relation" {
                        let a = (&lookup as &dyn SymbolLookup)
                            .declaration_type(arena, rows["Source"])
                            .unwrap();
                        let b = (&lookup as &dyn SymbolLookup)
                            .declaration_type(arena, rows["Target"])
                            .unwrap();
                        let expected = if case["supported"] == true {
                            case["admitted"].as_bool()
                        } else {
                            None
                        };
                        assert_eq!(
                            relation.argument(a, b),
                            expected,
                            "{} ({prefix}, rowless={rowless}): constructor variance",
                            case["name"]
                        );
                    } else {
                        let admitted = lookup.symbol_by_id(rows["Factory"]).is_some();
                        assert_eq!(
                            admitted,
                            case["admitted"]
                                .as_bool()
                                .unwrap_or(case["supported"] == true),
                            "{}: heritage admission",
                            case["name"]
                        );
                        let start = source.find("actual =").unwrap() as u32;
                        let values = lookup.source.unwrap();
                        let (&signature, &actual) = values
                            .initializers
                            .iter()
                            .find(|(id, _)| id.0.start == start)
                            .unwrap();
                        let selected = values
                            .constructor_calls
                            .get(&signature)
                            .and_then(Option::as_ref);
                        if case["supported"] == false {
                            assert!(
                                actual.is_none() && selected.is_none(),
                                "{}: rejected heritage or arguments cannot publish an initializer",
                                case["name"]
                            );
                            return;
                        }
                        assert_eq!(
                            actual,
                            lookup.field_type_id_of(rows["expected"]),
                            "{}: constructor result",
                            case["name"]
                        );
                        let selected = selected.unwrap();
                        let origin = selected.origins[selected.selected.unwrap()]
                            .as_ref()
                            .unwrap();
                        assert_eq!(
                            origin.span.start,
                            source.find(case["selected"].as_str().unwrap()).unwrap() as u32,
                            "{}: selected source signature",
                            case["name"]
                        );
                        assert_eq!(Some(origin.source), lookup.source.unwrap().identity);
                        let inventory = &lookup
                            .nominal_surface(rows["Factory"])
                            .unwrap()
                            .constructors;
                        let expected: Vec<_> = case["inventories"]["Factory"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|label| {
                                label["start"].as_u64().unwrap() as u32 + prefix.len() as u32
                            })
                            .collect();
                        assert_eq!(
                            inventory
                                .iter()
                                .map(|c| c.origin.signature.0.start)
                                .collect::<Vec<_>>(),
                            expected,
                            "{}: complete inherited signature inventory",
                            case["name"]
                        );
                        for constructor in inventory {
                            assert_eq!(
                                constructor.origin.declaration,
                                if rowless {
                                    None
                                } else {
                                    origin_rows
                                        .get(&constructor.origin.signature.0.start)
                                        .copied()
                                }
                            );
                        }
                        let label = &case["call"];
                        let expression = label["expression"].as_str().unwrap();
                        let selector =
                            source.find(expression).unwrap() + expression.rfind('.').unwrap() + 1;
                        let expected = origin_rows[&(source
                            .find(label["declaration"].as_str().unwrap())
                            .unwrap() as u32)];
                        let (target, result) = call(tree, &files[main], &ids, selector as u32)
                            .expect("constructor downstream call");
                        assert_eq!(
                            target, expected,
                            "{}: exact downstream declaration",
                            case["name"]
                        );
                        assert_eq!(
                            arena.format_type(result),
                            label["result"].as_str().unwrap(),
                            "{}: downstream result",
                            case["name"]
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
}
