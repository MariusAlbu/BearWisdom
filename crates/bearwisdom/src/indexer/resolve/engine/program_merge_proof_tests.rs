use super::*;

#[path = "program_query_proof_tests.rs"]
mod query_proofs;
use crate::indexer::resolve::ProjectContext;
use crate::indexer::{
    programs::{Program, ProgramSource, SourceScope},
    symbol_ids::SymbolIds,
};
use crate::types::ParsedFile;
use std::{collections::HashSet, sync::Arc};

pub(in crate::indexer::resolve::engine) fn parse(
    arena: &TypeArena,
    sources: &[(&str, &str)],
) -> Vec<ParsedFile> {
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
pub(in crate::indexer::resolve::engine) fn context(files: &[&ParsedFile]) -> ProjectContext {
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

#[test]
fn generic_augmentation_default_reaches_source_return_member_cascade() {
    use crate::indexer::resolve::engine::{
        contract::{FileContext, FlowCacheLookup},
        file_lookup::FileLookup,
        semantic_model::{SemanticModel, SolveOutcome},
        testkit,
    };
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            (
                "reader.d.ts",
                "interface Payload { touch(): void } interface Reader<T = Payload> { read(): T }",
            ),
            ("augmentation.d.ts", "interface Reader<T> {}"),
            (
                "main.ts",
                "declare const reader: Reader; function run() { reader.read().touch(); }",
            ),
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
    let reader = owner(&ids, &files[0], "Reader");
    let target = owner(&ids, &files[0], "touch");
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    let selected = tree.program_lookup("main.ts").unwrap();
    assert!(
        selected.symbol_by_id(reader).is_some(),
        "a legal omitted default must not reject its provider"
    );
    let file = &files[2];
    let lookup = FileLookup::for_file(&tree, file, &ids);
    let context = FileContext {
        file_path: file.path.clone(),
        language: "typescript".into(),
        imports: vec![],
        file_namespace: None,
    };
    let model = SemanticModel::production();
    let mut found = false;
    for reference in file
        .refs
        .iter()
        .filter(|r| r.kind == crate::types::EdgeKind::Calls)
    {
        let mut site = testkit::ref_ctx(
            reference,
            &file.symbols[reference.source_symbol_index],
            vec![],
        );
        site.source_symbol_id = ids.row_id(&file.path, reference.source_symbol_index);
        lookup.set_cursor(reference.byte_offset);
        match model.get_symbol_info(
            &site,
            &context,
            &lookup,
            &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
        ) {
            SolveOutcome::Resolved(info) => {
                found |= info.target_symbol_id == target;
            }
            SolveOutcome::Unresolved(cause) => {
                panic!("defaulted generic return cascade did not resolve: {cause:?}")
            }
            SolveOutcome::Drained => panic!("defaulted generic return cascade was drained"),
        }
    }
    assert!(
        found,
        "default argument must reach the exact source Payload member"
    );
}

#[test]
fn merge_admission_matches_compiler_compatibility_evidence_fresh_and_cold() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/merge_compatibility_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        let arena = Arc::new(TypeArena::new());
        let files = parse(&arena, &[("main.ts", case["source"].as_str().unwrap()),
            ("healthy.d.ts", "interface Healthy { [index: number]: string; } interface Healthy { read(): string; }")]);
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &files,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let rows: Vec<_> = files[0]
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.name == "Catalog")
            .map(|(slot, _)| ids.row_id("main.ts", slot).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        let healthy = ids.row_id("healthy.d.ts", 0).unwrap();
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
                "{}: unrelated merge was lost",
                case["name"]
            );
            for &row in &rows {
                assert_eq!(
                    lookup.symbol_by_id(row).is_some(),
                    case["admitted"].as_bool().unwrap(),
                    "{}",
                    case["name"]
                );
            }
            if case["admitted"] == true {
                assert_eq!(
                    lookup.canonical_decl_id(rows[0]),
                    lookup.canonical_decl_id(rows[1])
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
fn member_identity_domains_are_disjoint() {
    assert!(MemberKey::Call != MemberKey::Construct);
    assert!(MemberKey::Index(Intrinsic::String) != MemberKey::Index(Intrinsic::Number));
}

const KEYS: &str =
    "declare const keys: Keys; interface Keys { readonly tag: unique symbol; value: string; }";
const DEPENDENT: &str = "interface Dep { [keys.tag](): void; } interface Dep { keep(): void; }";
const HEALTHY: &str =
    "interface Healthy { [index: number]: string; } interface Healthy { read(): string; }";

pub(in crate::indexer::resolve::engine) fn owner(
    ids: &SymbolIds,
    file: &ParsedFile,
    name: &str,
) -> i64 {
    ids.row_id(
        &file.path,
        file.symbols.iter().position(|s| s.name == name).unwrap(),
    )
    .unwrap()
}
fn check_dependencies(tree: &Compilation, keys: i64, dependent: i64, healthy: i64, admitted: bool) {
    let lookup = tree.program_lookup("dep.d.ts").unwrap();
    assert_eq!(
        lookup.symbol_by_id(keys).is_some(),
        admitted,
        "provider admission"
    );
    assert_eq!(
        lookup.symbol_by_id(dependent).is_some(),
        admitted,
        "dependent admission"
    );
    assert!(
        lookup.symbol_by_id(healthy).is_some(),
        "unrelated candidate must survive rejection rounds"
    );
    let start = DEPENDENT.find("[keys.tag]").unwrap() as u32;
    assert_eq!(
        lookup
            .computed_key(crate::types::SourceSpan {
                start,
                end: start + 10
            })
            .flatten()
            .is_some(),
        admitted
    );
}

#[test]
fn rejected_merge_invalidates_computed_dependents_after_provider_edit_and_readmits_after_deletion()
{
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            ("keys.d.ts", KEYS),
            ("extra.d.ts", "interface Keys { value: string; }"),
            ("dep.d.ts", DEPENDENT),
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
    let (keys, dependent, healthy) = (
        owner(&ids, &files[0], "Keys"),
        owner(&ids, &files[2], "Dep"),
        owner(&ids, &files[3], "Healthy"),
    );
    let check =
        |tree: &Compilation, admitted| check_dependencies(tree, keys, dependent, healthy, admitted);
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    check(&tree, true);
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(
        &arena,
        &[("extra.d.ts", "interface Keys { value: number; }")],
    );
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
    let mut missing = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    missing.ingest_from_db(db.conn());
    assert!(
        missing
            .program_lookup("dep.d.ts")
            .unwrap()
            .symbol_by_id(dependent)
            .is_none(),
        "stale source membership fences the program"
    );
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
fn merge_proofs_do_not_leak_between_overlapping_programs_or_stale_sources() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            ("keys.d.ts", KEYS),
            ("valid.d.ts", "interface Keys { value: string; }"),
            ("invalid.d.ts", "interface Keys { value: number; }"),
            ("left.ts", DEPENDENT),
            ("right.ts", DEPENDENT),
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
    let keys = owner(&ids, &files[0], "Keys");
    let left = owner(&ids, &files[3], "Dep");
    let right = owner(&ids, &files[4], "Dep");
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
    let stale = parse(&arena, &[("left.ts", &format!("{DEPENDENT}\n"))]).remove(0);
    let check = |tree: &Compilation| {
        let a = tree.program_lookup("left.ts").unwrap();
        let b = tree.program_lookup("right.ts").unwrap();
        assert!(a.symbol_by_id(keys).is_some());
        assert!(a.symbol_by_id(left).is_some());
        assert!(a.symbol_by_id(right).is_none());
        assert!(b.symbol_by_id(keys).is_none());
        assert!(b.symbol_by_id(right).is_none());
        assert!(b.symbol_by_id(left).is_none());
        assert!(
            tree.program_lookup("keys.d.ts")
                .unwrap()
                .symbol_by_id(keys)
                .is_none(),
            "shared source needs program selection"
        );
        assert!(tree
            .source_program_lookup(&stale)
            .unwrap()
            .symbol_by_id(left)
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

#[test]
fn merge_proof_preserves_rowless_member_recipes_through_portable_and_cold_providers() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let original = TypeArena::new();
    let source = "function discard() {} interface Catalog { value: string; } interface Catalog { value: string; read(): string; }";
    let mut files = parse(&original, &[("provider.d.ts", source)]);
    let discarded = files[0]
        .symbols
        .iter()
        .position(|s| s.name == "discard")
        .unwrap();
    for property in files[0].symbols.iter_mut().filter(|s| s.name == "value") {
        property.parent_index = Some(discarded);
    }
    reduce_to_contract(&mut files[0]);
    assert!(!files[0].symbols.iter().any(|s| s.name == "value"));
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
    assert!(!file.symbols.iter().any(|s| s.name == "value"));
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&file),
        "external",
        Some(&arena),
    )
    .unwrap();
    let rows: Vec<_> = file
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.name == "Catalog")
        .map(|(slot, _)| ids.row_id(&file.path, slot).unwrap())
        .collect();
    let tree = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&file])),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("provider.d.ts").unwrap();
        assert_eq!(rows.len(), 2);
        for &row in &rows {
            assert!(lookup.symbol_by_id(row).is_some());
        }
        assert_eq!(
            lookup.canonical_decl_id(rows[0]),
            lookup.canonical_decl_id(rows[1])
        );
    };
    check(&tree);
    let before =
        serde_json::to_value(super::super::super::program_input::capture(&file, &ids)).unwrap();
    for symbol in &mut file.symbols {
        symbol.name = "poison".into();
        symbol.qualified_name = "poison.display".into();
        symbol.signature = None;
    }
    assert_eq!(
        serde_json::to_value(super::super::super::program_input::capture(&file, &ids)).unwrap(),
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
fn proved_property_rows_share_identity_without_collapsing_overloads_or_namesakes() {
    let arena = Arc::new(TypeArena::new());
    let source = "interface Catalog<T> { value: T; choose(x: string): string; } interface Catalog<T> { value: T; choose(x: number): number; } interface Other<T> { value: T; }";
    let files = parse(&arena, &[("main.ts", source)]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let rows = |name: &str| {
        files[0]
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.name == name)
            .map(|(slot, _)| ids.row_id("main.ts", slot).unwrap())
            .collect::<Vec<_>>()
    };
    let properties = rows("value");
    let overloads = rows("choose");
    let catalog = owner(&ids, &files[0], "Catalog");
    assert_eq!(properties.len(), 3);
    assert_eq!(overloads.len(), 2);
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0]])),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        assert_eq!(
            lookup.canonical_decl_id(properties[0]),
            lookup.canonical_decl_id(properties[1])
        );
        assert_ne!(
            lookup.canonical_decl_id(properties[0]),
            lookup.canonical_decl_id(properties[2])
        );
        assert_ne!(
            lookup.canonical_decl_id(overloads[0]),
            lookup.canonical_decl_id(overloads[1])
        );
        assert!(
            properties
                .iter()
                .chain(&overloads)
                .all(|&row| lookup.symbol_by_id(row).is_some()),
            "physical navigation parts remain available"
        );
        assert_eq!(
            lookup.field_type_id_of(properties[0]),
            lookup.field_type_id_of(properties[1])
        );
        let index = lookup.member_index().unwrap();
        use crate::indexer::resolve::engine::member_selection::{select, Selection};
        assert_eq!(
            select(&lookup, catalog, index.name("value").unwrap(), &|_| true),
            Selection::Unique(properties[0])
        );
        assert_eq!(
            select(&lookup, catalog, index.name("choose").unwrap(), &|_| true),
            Selection::Ambiguous
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
#[ignore = "Read-only first-failure provenance in private real-library merge views"]
fn diagnose_pinned_private_merge_rejection_facts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: crate::resolution_oracle::project::ProjectManifest =
        serde_json::from_slice(
            &std::fs::read(root.join(
                "resolution-documents/2026-09-07-query-core-configured-compiler-manifest.json",
            ))
            .unwrap(),
        )
        .unwrap();
    manifest.verify_inputs().unwrap();
    let arena = Arc::new(TypeArena::new());
    let files: Vec<_> = manifest
        .files
        .iter()
        .map(|file| {
            crate::indexer::parse_file::parse_file_with_arena(
                &crate::walker::WalkedFile {
                    relative_path: file.index_path.clone(),
                    absolute_path: file.path.clone(),
                    language: "typescript",
                },
                crate::languages::default_registry(),
                &arena,
            )
            .unwrap()
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
    let mut config = crate::indexer::project_context::build_project_context(&manifest.root);
    config.programs = Some(vec![Program {
        key: "diagnostic".into(),
        fingerprint: "pinned-diagnostic".into(),
        complete: true,
        callable_policy: None,
        compiler_intrinsics: None,
        source_binding_order: None,
        sources: manifest
            .files
            .iter()
            .map(|file| ProgramSource {
                path: file.index_path.clone(),
                content_hash: file.sha256.clone(),
                scope: file.source_scope.unwrap(),
            })
            .collect(),
    }]);
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    let modules = tree._test_program_modules();
    let program = modules.programs.program("diagnostic").unwrap();
    let sources = modules.programs.sources(program);
    let mut allowed: FxHashSet<_> = modules
        .programs
        .pending(program)
        .iter()
        .map(|p| p.key)
        .collect();
    for round in 0..=allowed.len() {
        let staged = modules.programs.staged(program, &allowed);
        let view = View::build(
            program,
            &sources,
            modules,
            &tree,
            &arena,
            &staged,
            &Default::default(),
        );
        let mut invalid = Vec::new();
        for candidate in modules
            .programs
            .pending(program)
            .iter()
            .filter(|p| allowed.contains(&p.key))
        {
            let accepted = compatible(candidate, &sources, modules, &view, &tree, &arena).is_some();
            if !accepted {
                invalid.push(candidate.key);
            }
            if !matches!(
                candidate.parts[0].1.name.as_str(),
                "Set" | "Map" | "Promise" | "PromiseConstructor" | "SymbolConstructor"
            ) {
                continue;
            }
            println!(
                "MERGE round={round} group={} accepted={accepted}",
                candidate.parts[0].1.name
            );
            for (source, part) in &candidate.parts {
                let (path, _) = sources.iter().find(|(_, id)| id == source).unwrap();
                let input = modules.inputs[path]
                    .globals
                    .as_ref()
                    .unwrap()
                    .types
                    .as_ref()
                    .unwrap();
                let lookup = Lookup {
                    tree: &tree,
                    view: &view,
                    source: view.sources.get(source),
                };
                let names = input
                    .names
                    .iter()
                    .filter_map(|(name, spelling)| {
                        view.members.name(spelling).map(|id| (*name, id))
                    })
                    .collect();
                for member in part.surface.as_deref().unwrap_or(&[]) {
                    if fact(member, &names, &lookup, &arena).is_some() {
                        continue;
                    }
                    let text = files
                        .iter()
                        .find(|file| &file.path == path)
                        .and_then(|file| file.content.as_ref())
                        .and_then(|content| {
                            content.get(member.span.start as usize..member.span.end as usize)
                        });
                    println!("MISSING {} source={path} member={member:?} signature={:?} key={:?} text={text:?}", part.name,
                        lookup.signature(source_signatures::SignatureId(member.span)), member.key_span.and_then(|span| lookup.computed_key(span)));
                    break;
                }
            }
        }
        if invalid.is_empty() {
            break;
        }
        for key in invalid {
            allowed.remove(&key);
        }
    }
    manifest.verify_inputs().unwrap();
}

#[test]
#[ignore = "Read-only source-owned structural heritage barriers in the pinned compiler cohort"]
fn diagnose_pinned_structural_heritage_barriers() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: crate::resolution_oracle::project::ProjectManifest =
        serde_json::from_slice(
            &std::fs::read(root.join(
                "resolution-documents/2026-09-07-query-core-configured-compiler-manifest.json",
            ))
            .unwrap(),
        )
        .unwrap();
    manifest.verify_inputs().unwrap();
    let arena = Arc::new(TypeArena::new());
    let files: Vec<_> = manifest
        .files
        .iter()
        .map(|file| {
            crate::indexer::parse_file::parse_file_with_arena(
                &crate::walker::WalkedFile {
                    relative_path: file.index_path.clone(),
                    absolute_path: file.path.clone(),
                    language: "typescript",
                },
                crate::languages::default_registry(),
                &arena,
            )
            .unwrap()
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
    let mut config = crate::indexer::project_context::build_project_context(&manifest.root);
    config.programs = Some(vec![Program {
        key: "diagnostic".into(),
        fingerprint: "pinned-structural-diagnostic".into(),
        complete: true,
        callable_policy: None,
        compiler_intrinsics: None,
        source_binding_order: None,
        sources: manifest
            .files
            .iter()
            .map(|file| ProgramSource {
                path: file.index_path.clone(),
                content_hash: file.sha256.clone(),
                scope: file.source_scope.unwrap(),
            })
            .collect(),
    }]);
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    let modules = tree._test_program_modules();
    let program = modules.programs.program("diagnostic").unwrap();
    let sources = modules.programs.sources(program);
    let allowed = modules
        .programs
        .pending(program)
        .iter()
        .map(|p| p.key)
        .collect();
    let staged = modules.programs.staged(program, &allowed);
    let view = View::build(
        program,
        &sources,
        modules,
        &tree,
        &arena,
        &staged,
        &Default::default(),
    );
    for (path, source) in &sources {
        let input = modules.inputs[path]
            .globals
            .as_ref()
            .unwrap()
            .types
            .as_ref()
            .unwrap();
        let lookup = Lookup {
            tree: &tree,
            view: &view,
            source: view.sources.get(source),
        };
        let relation = types::Relation {
            lookup: &lookup,
            arena: &arena,
        };
        for part in input.interfaces.iter().filter(|part| {
            tree.symbol_by_id(part.owner)
                .is_some_and(|symbol| symbol.name == "QueryObserverOptions")
        }) {
            for base in part.bases.as_ref().unwrap() {
                let raw = base.materialize(&lookup, &arena, path);
                println!(
                    "HERITAGE source={path} owner={} raw={} canonical={:?}",
                    part.owner,
                    arena.format_type(raw),
                    relation.canonical(raw, 0).map(|ty| arena.format_type(ty))
                );
                if let Type::Apply { args, .. } = arena.get(raw) {
                    for argument in args {
                        let keys = arena.intern(Type::Operator(
                            crate::type_checker::core::types::TypeOperator::KeyOf(argument),
                        ));
                        println!(
                            "ARG type={} canonical={:?} keys={:?}",
                            arena.format_type(argument),
                            relation
                                .canonical(argument, 0)
                                .map(|ty| arena.format_type(ty)),
                            relation.canonical(keys, 0).map(|ty| arena.format_type(ty))
                        );
                        if let Some(owner) =
                            crate::indexer::resolve::engine::head_decl::head_decl_id(
                                &arena, argument,
                            )
                        {
                            let info = lookup.canonical_type_info(owner).unwrap();
                            for &parameter in &info.generic_param_ids {
                                let constraint = lookup.generic_constraint(parameter);
                                println!(
                                    "CONSTRAINT parameter={parameter:?} type={:?} canonical={:?}",
                                    constraint.map(|c| c.map(|ty| arena.format_type(ty))),
                                    constraint
                                        .flatten()
                                        .and_then(|ty| relation.canonical(ty, 0))
                                        .map(|ty| arena.format_type(ty))
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    manifest.verify_inputs().unwrap();
}
