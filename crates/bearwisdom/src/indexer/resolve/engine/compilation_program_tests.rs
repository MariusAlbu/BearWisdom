use super::super::program_graph::{Graph, Result};
use super::*;
use crate::indexer::programs::{Program, ProgramSource, SourceScope};

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
fn program(key: &str, files: &[&ParsedFile]) -> Program {
    Program {
        key: key.into(),
        fingerprint: key.into(),
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
    }
}
fn targets(graph: &Graph, key: &str, spelling: &str, type_space: bool) -> Option<Vec<i64>> {
    let program = graph.program(key)?;
    let name = graph.name(program, spelling)?;
    let Result::Bound(group) = graph.global(program, name, type_space) else {
        return None;
    };
    Some(
        graph
            .group(group)?
            .parts
            .iter()
            .map(|p| p.declaration)
            .collect(),
    )
}

#[test]
fn selected_files_use_isolated_signatures_and_shared_source_requires_selection() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            ("shared.d.ts", "interface Catalog<T> { first(): T; }"),
            (
                "a.d.ts",
                "interface Catalog<T> { forEach(callback: (item: T) => void): void; }",
            ),
            ("b.d.ts", "interface Catalog<T> { last(): T; }"),
            (
                "a.ts",
                "export {}; class DocA {} function run(list: Catalog<DocA>) { list.first(); }",
            ),
            (
                "b.ts",
                "export {}; class DocB {} function run(list: Catalog<DocB>) { list.first(); }",
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
    let context = ProjectContext {
        programs: Some(vec![
            program("a", &[&files[0], &files[1], &files[3]]),
            program("b", &[&files[0], &files[2], &files[4]]),
        ]),
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &HashSet::new(),
    );
    let owner = ids.row_id("shared.d.ts", 0).unwrap();
    let check = |tree: &Compilation| {
        let a = tree.program_lookup("a.ts").unwrap();
        let b = tree.program_lookup("b.ts").unwrap();
        assert_ne!(a.nominal_context(), b.nominal_context());
        assert_ne!(
            a.canonical_type_info(owner).unwrap().generic_param_ids,
            b.canonical_type_info(owner).unwrap().generic_param_ids
        );
        let first = a.member_index().unwrap().name("first").unwrap();
        assert_eq!(
            a.member_index().unwrap().candidates(owner, first),
            b.member_index().unwrap().candidates(owner, first)
        );
        let callback = a.member_index().unwrap().name("forEach").unwrap();
        assert_eq!(
            a.member_index().unwrap().candidates(owner, callback).len(),
            1
        );
        assert!(b
            .member_index()
            .unwrap()
            .candidates(owner, callback)
            .is_empty());
        let overlapping = tree.program_lookup("shared.d.ts").unwrap();
        assert!(overlapping.nominal_context().is_some());
        assert!(
            overlapping.symbol_by_id(owner).is_none(),
            "overlap must not pick a program or use workspace signatures"
        );
        let local = |file: &ParsedFile| {
            let lookup = super::super::file_lookup::FileLookup::for_file(tree, file, &ids);
            let reference = file
                .refs
                .iter()
                .find(|r| r.kind == EdgeKind::Calls)
                .unwrap();
            lookup.set_cursor(reference.byte_offset);
            lookup
                .local_reference(reference.byte_offset)
                .unwrap()
                .value_type
                .unwrap()
        };
        let ty_a = local(&files[3]);
        let ty_b = local(&files[4]);
        let arena = tree.type_arena().unwrap();
        assert!(arena.accepts_nominal_context(ty_a, a.nominal_context()));
        assert!(!arena.accepts_nominal_context(ty_a, b.nominal_context()));
        assert_ne!(ty_a, ty_b);
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
fn stale_file_capture_cannot_interpret_current_program_source_ids() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            ("api.d.ts", "interface Catalog<T> { first(): T; }"),
            (
                "main.ts",
                "export {}; function run(list: Catalog<string>) { list.first(); }",
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
    let context = ProjectContext {
        programs: Some(vec![program("only", &[&files[0], &files[1]])]),
        ..Default::default()
    };
    let tree =
        Compilation::build_with_context(&files, &ids, arena, Some(&context), &HashSet::new());
    let owner = ids.row_id("api.d.ts", 0).unwrap();
    let mut stale = files.into_iter().nth(1).unwrap();
    stale.content_hash = "different-source-snapshot".into();
    let lookup = super::super::file_lookup::FileLookup::for_file(&tree, &stale, &ids);
    assert!(lookup.nominal_context().is_some());
    assert!(lookup.canonical_type_info(owner).is_none());
    for reference in &stale.refs {
        if let Some(local) = lookup.local_reference(reference.byte_offset) {
            assert!(local.declaration.is_none());
        }
    }
}

#[test]
fn configured_generic_function_signatures_survive_filtered_portable_cache() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let arena = TypeArena::new();
    let mut files = parse(
        &arena,
        &[("provider.d.ts", "declare function make<T>(value: T): T;")],
    );
    let source = &mut files[0];
    reduce_to_contract(source);
    assert_eq!(source.symbols[0].start_col, 0);
    let payload = serde_json::to_string(&CachedParse::from_parsed(source, &arena)).unwrap();
    let other = Arc::new(TypeArena::new());
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut restored = cached.into_parsed(
        &other,
        "provider.d.ts",
        &source.content_hash,
        source.size,
        None,
    );
    restored.content = source.content.clone();
    reduce_to_contract(&mut restored);
    let context = ProjectContext {
        programs: Some(vec![program("only", &[&restored])]),
        ..Default::default()
    };
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&restored),
        "external",
        Some(&other),
    )
    .unwrap();
    let tree = Compilation::build_with_context(
        &[restored],
        &ids,
        Arc::clone(&other),
        Some(&context),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("provider.d.ts").unwrap();
        let info = lookup
            .canonical_type_info(ids.row_id("provider.d.ts", 0).unwrap())
            .unwrap();
        assert_eq!(info.generic_param_ids.len(), 1);
        let expected = tree
            .type_arena()
            .unwrap()
            .generic_type(info.generic_param_ids[0]);
        assert_eq!(info.return_type_id, Some(expected));
        assert_eq!(info.parameter_type_ids, Some(vec![expected]));
    };
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let mut cold = Compilation::build(&[], &SymbolIds::default(), other);
    cold.ingest_from_db(db.conn());
    check(&cold);
}

#[test]
fn rejected_global_group_cannot_be_revived_by_a_provider_local_type_recipe() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            (
                "a.d.ts",
                "interface Catalog<T> { first(): T; } declare var selected: Catalog<string>;",
            ),
            ("b.d.ts", "interface Catalog<U> { last(): U; }"),
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
    let context = ProjectContext {
        programs: Some(vec![program("only", &[&files[0], &files[1]])]),
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &HashSet::new(),
    );
    let lookup = tree.program_lookup("a.d.ts").unwrap();
    let owner = ids.row_id("a.d.ts", 0).unwrap();
    assert!(
        lookup.symbol_by_id(owner).is_none(),
        "rejected merge is not a usable program nominal"
    );
    let variable = files[0]
        .symbols
        .iter()
        .position(|s| s.kind == SymbolKind::Variable)
        .unwrap();
    let ty = lookup
        .field_type_id_of(ids.row_id("a.d.ts", variable).unwrap())
        .unwrap();
    assert!(super::super::head_decl::head_decl_id(&arena, ty).is_none());
}

#[test]
fn configured_global_groups_reconstruct_from_real_source_without_cross_program_merges() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[
        ("shared.d.ts", "interface Catalog<T> { first(): T; }"),
        ("a.d.ts", "interface Catalog<T> { forEach(callback: (item: T) => void): void; }"),
        ("b.d.ts", "interface Catalog<T> { last(): T; }"),
        ("module.ts", "export interface Catalog<T> { unrelated(): T; }"),
        ("values.d.ts", "interface Deferred<T> { then(): T; } interface DeferredConstructor { reject(): Deferred<never>; } declare var Deferred: DeferredConstructor;"),
    ]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let row = |file: usize, name: &str, kind: crate::types::SymbolKind| {
        ids.row_id(
            &files[file].path,
            files[file]
                .symbols
                .iter()
                .position(|s| s.name == name && s.kind == kind)
                .unwrap(),
        )
        .unwrap()
    };
    let a = vec![
        row(0, "Catalog", crate::types::SymbolKind::Interface),
        row(1, "Catalog", crate::types::SymbolKind::Interface),
    ];
    let b = vec![a[0], row(2, "Catalog", crate::types::SymbolKind::Interface)];
    let context = ProjectContext {
        programs: Some(vec![
            program("a", &[&files[0], &files[1], &files[3], &files[4]]),
            program("b", &[&files[0], &files[2], &files[3]]),
        ]),
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let graph = &tree.modules.programs;
        assert_eq!(targets(graph, "a", "Catalog", true), Some(a.clone()));
        assert_eq!(targets(graph, "b", "Catalog", true), Some(b.clone()));
        assert_eq!(
            targets(graph, "a", "Deferred", false),
            Some(vec![row(4, "Deferred", crate::types::SymbolKind::Variable)])
        );
        assert_eq!(
            targets(graph, "a", "Deferred", true),
            Some(vec![row(
                4,
                "Deferred",
                crate::types::SymbolKind::Interface
            )])
        );
        assert!(targets(graph, "b", "Deferred", true).is_none());
        assert_ne!(
            tree.canonical_decl_id(a[0]),
            tree.canonical_decl_id(a[1]),
            "physical workspace identities were not globally rewritten"
        );
    };
    check(&tree);
    let graph = &tree.modules.programs;
    let context_a = graph.nominal_context(graph.program("a").unwrap()).unwrap();
    let context_b = graph.nominal_context(graph.program("b").unwrap()).unwrap();
    let type_a = arena.decl_in(context_a, "Catalog", a[0]);
    let type_b = arena.decl_in(context_b, "Catalog", a[0]);
    assert_ne!(
        type_a, type_b,
        "the same parsed row is not the same configured nominal"
    );
    tree.persist_type_info(db.conn()).unwrap();
    let fresh_arena = Arc::new(TypeArena::new());
    fresh_arena.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&fresh_arena));
    cold.ingest_from_db(db.conn());
    check(&cold);
    let rebuilt_a = cold
        .modules
        .programs
        .nominal_context(cold.modules.programs.program("a").unwrap())
        .unwrap();
    assert_ne!(rebuilt_a, context_a);
    assert!(
        !fresh_arena.accepts_nominal_context(type_a, Some(rebuilt_a)),
        "stored contexts must be rebound, not revived"
    );
    db.conn()
        .execute("DELETE FROM files WHERE path='a.d.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), fresh_arena);
    deleted.ingest_from_db(db.conn());
    assert!(targets(&deleted.modules.programs, "a", "Catalog", true).is_none());
    assert_eq!(
        targets(&deleted.modules.programs, "b", "Catalog", true),
        Some(b)
    );
    assert!(deleted
        .program_lookup("a.d.ts")
        .unwrap()
        .symbol_by_id(a[0])
        .is_none());
    assert!(deleted
        .program_lookup("b.d.ts")
        .unwrap()
        .canonical_type_info(a[0])
        .is_some());
}

#[test]
fn global_capture_survives_filtered_portable_sources_and_ignores_display_spelling() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let arena = TypeArena::new();
    let mut files = parse(
        &arena,
        &[(
            "provider.d.ts",
            "interface Catalog<T> { first(): T; } function run() { class Hidden {} }",
        )],
    );
    let source = &mut files[0];
    reduce_to_contract(source);
    let payload = serde_json::to_string(&CachedParse::from_parsed(source, &arena)).unwrap();
    let other = Arc::new(TypeArena::new());
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut restored = cached.into_parsed(
        &other,
        "provider.d.ts",
        &source.content_hash,
        source.size,
        None,
    );
    restored.content = source.content.clone();
    reduce_to_contract(&mut restored);
    let context = ProjectContext {
        programs: Some(vec![program("only", &[&restored])]),
        ..Default::default()
    };
    let db = crate::Database::open_in_memory().unwrap();
    let original_slot = restored
        .symbols
        .iter()
        .position(|s| s.name == "Catalog")
        .unwrap();
    for symbol in &mut restored.symbols {
        symbol.name = "poisoned".into();
        symbol.qualified_name = "poisoned".into();
    }
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&restored),
        "external",
        Some(&other),
    )
    .unwrap();
    let tree =
        Compilation::build_with_context(&[restored], &ids, other, Some(&context), &HashSet::new());
    assert_eq!(
        targets(&tree.modules.programs, "only", "Catalog", true),
        Some(vec![ids.row_id("provider.d.ts", original_slot).unwrap()])
    );
    assert!(targets(&tree.modules.programs, "only", "Hidden", true).is_none());
}

#[test]
fn source_edits_and_configuration_changes_rebind_unchanged_program_members() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[
            ("shared.d.ts", "interface Catalog<T> { first(): T; }"),
            ("provider.d.ts", "interface Catalog<T> { last(): T; }"),
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
    let mut context = ProjectContext {
        programs: Some(vec![program("only", &[&files[0], &files[1]])]),
        ..Default::default()
    };
    let mut tree = Compilation::build_with_context(
        &files[..1],
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &HashSet::new(),
    );
    assert!(
        targets(&tree.modules.programs, "only", "Catalog", true).is_none(),
        "missing late provider is not absence of a competitor"
    );
    tree.ingest(&files[1..], &ids, &HashSet::new());
    let initial = targets(&tree.modules.programs, "only", "Catalog", true).unwrap();
    assert_eq!(initial.len(), 2);
    let view = tree.program_lookup("shared.d.ts").unwrap();
    let last = view.member_index().unwrap().name("last").unwrap();
    assert_eq!(
        view.member_index()
            .unwrap()
            .candidates(initial[0], last)
            .len(),
        1
    );
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(
        &arena,
        &[(
            "provider.d.ts",
            "export interface Catalog<T> { last(): T; }",
        )],
    );
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    context.programs = Some(vec![program("only", &[&files[0], &changed[0]])]);
    context.programs.as_mut().unwrap()[0].fingerprint = "changed-config".into();
    let mut incremental = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&context),
        &HashSet::new(),
    );
    incremental.ingest_from_db(db.conn());
    assert_eq!(
        targets(&incremental.modules.programs, "only", "Catalog", true),
        Some(vec![initial[0]])
    );
    let view = incremental.program_lookup("shared.d.ts").unwrap();
    let last = view.member_index().unwrap().name("last").unwrap();
    assert!(
        view.member_index()
            .unwrap()
            .candidates(initial[0], last)
            .is_empty(),
        "exported namesake leaked into unchanged global signatures"
    );
    incremental.persist_type_info(db.conn()).unwrap();
    let mut cold = Compilation::build(&[], &SymbolIds::default(), arena);
    cold.ingest_from_db(db.conn());
    assert_eq!(
        targets(&cold.modules.programs, "only", "Catalog", true),
        Some(vec![initial[0]])
    );
    assert_eq!(cold.modules.programs.configuration, context.programs);
    let view = cold.program_lookup("shared.d.ts").unwrap();
    let last = view.member_index().unwrap().name("last").unwrap();
    assert!(view
        .member_index()
        .unwrap()
        .candidates(initial[0], last)
        .is_empty());
}

#[test]
fn ambient_provider_actual_edits_late_arrival_and_deletion_refresh_imported_signatures() {
    use crate::indexer::lexical::ScopeId;
    for (before, after, consumer, namespace) in [
        ("declare module 'provider' { class Doc { left(): void; } function make(): Doc; }",
         "declare module 'provider' { class Doc { right(): void; } function make(): Doc; }",
         "import { make } from 'provider'; make().left();", false),
        ("declare module 'provider' { function make(): make.Doc; namespace make { class Doc { left(): void; } function extra(): Doc; } import Factory = make; export = Factory; }",
         "declare module 'provider' { function make(): make.Doc; namespace make { class Doc { right(): void; } function extra(): Doc; } import Factory = make; export = Factory; }",
         "import make = require('provider'); make().left(); make.extra().left();", true),
    ] {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[
        ("provider.d.ts", before), ("main.ts", consumer),
    ]);
    let graph = files[1].flow.lexical.as_ref().unwrap();
    let binding = graph.lookup(ScopeId(0), graph.name_id("make").unwrap()).unwrap();
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(&db, &files, "internal", Some(&arena)).unwrap();
    let context = |sources: &[&ParsedFile]| ProjectContext { programs: Some(vec![program("only", sources)]), ..Default::default() };
    let original_context = context(&[&files[0], &files[1]]);
    let mut tree = Compilation::build_with_context(&files[1..], &ids, Arc::clone(&arena), Some(&original_context), &HashSet::new());
    assert!(tree.program_lookup("main.ts").unwrap().bound_import("main.ts", binding, false).is_none());
    tree.ingest(&files[..1], &ids, &HashSet::new());
    let check = |tree: &Compilation, source: &ParsedFile, source_ids: &SymbolIds, member: &str, absent: &str| {
        let row = |name| source_ids.row_id(&source.path, source.symbols.iter().position(|s| s.name == name).unwrap()).unwrap();
        let lookup = tree.program_lookup("main.ts").unwrap();
        assert_eq!(lookup.bound_import("main.ts", binding, false), Some(row("make")));
        assert_eq!(lookup.bound_import_namespace("main.ts", binding), namespace);
        if namespace {
            let selector = graph.module.members[&(consumer.find("extra()").unwrap() as u32)];
            assert_eq!(lookup.bound_import("main.ts", selector, false), Some(row("extra")));
            let result = lookup.return_type_id_of(row("extra")).unwrap();
            assert_eq!(super::super::head_decl::head_decl_id(tree.type_arena().unwrap(), result), Some(row("Doc")));
        }
        let result = lookup.return_type_id_of(row("make")).unwrap();
        let arena = tree.type_arena().unwrap();
        assert!(arena.accepts_nominal_context(result, lookup.nominal_context()));
        assert_eq!(super::super::head_decl::head_decl_id(arena, result), Some(row("Doc")));
        let index = lookup.member_index().unwrap();
        assert_eq!(index.candidates(row("Doc"), index.name(member).unwrap()), &[row(member)]);
        assert!(index.name(absent).is_none_or(|name| index.candidates(row("Doc"), name).is_empty()));
    };
    check(&tree, &files[0], &ids, "left", "right"); tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(&arena, &[("provider.d.ts", after)]);
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(&db, &changed, "internal", Some(&arena)).unwrap();
    let changed_context = context(&[&changed[0], &files[1]]);
    let mut edited = Compilation::build_with_context(&changed, &changed_ids, Arc::clone(&arena), Some(&changed_context), &HashSet::new());
    edited.ingest_from_db(db.conn()); check(&edited, &changed[0], &changed_ids, "right", "left");
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new()); restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn()); check(&cold, &changed[0], &changed_ids, "right", "left");
    db.conn().execute("DELETE FROM files WHERE path='provider.d.ts'", []).unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), restored); deleted.ingest_from_db(db.conn());
    assert!(deleted.program_lookup("main.ts").unwrap().bound_import("main.ts", binding, false).is_none());
    assert!(!deleted.program_lookup("main.ts").unwrap().bound_import_namespace("main.ts", binding));
    }
}

#[test]
fn ambient_import_and_nested_global_signatures_survive_filtered_portable_source_caches() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse, lexical::ScopeId,
    };
    for (source, consumer, namespace) in [
        ("declare module 'provider' { class Doc { touch(): void; } function make(): Doc; global { interface Catalog { read(): Doc; } } }",
         "import { make } from 'provider'; make().touch();", false),
        ("declare module 'provider' { function make(): make.Doc; namespace make { class Doc { touch(): void; } function extra(): Doc; } import Factory = make; global { interface Catalog { read(): make.Doc; } } export = Factory; }",
         "import make = require('provider'); make().touch(); make.extra().touch();", true),
    ] {
    let original = TypeArena::new();
    let mut providers = parse(&original, &[("provider.d.ts", source)]);
    reduce_to_contract(&mut providers[0]);
    let payload = serde_json::to_string(&CachedParse::from_parsed(&providers[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut provider = cached.into_parsed(&arena, "provider.d.ts", &providers[0].content_hash, providers[0].size, None);
    provider.content = providers[0].content.clone(); reduce_to_contract(&mut provider);
    let mut files = parse(&arena, &[("main.ts", consumer)]);
    files.push(provider);
    let graph = files[0].flow.lexical.as_ref().unwrap();
    let binding = graph.lookup(ScopeId(0), graph.name_id("make").unwrap()).unwrap();
    let row_slot = |name| files[1].symbols.iter().position(|s| s.name == name).unwrap();
    let slots = [row_slot("Doc"), row_slot("make"), row_slot("Catalog"), row_slot("read")];
    for symbol in &mut files[1].symbols { symbol.name = "poisoned".into(); symbol.qualified_name = "poisoned".into(); symbol.signature = None; }
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(&db, &files, "internal", Some(&arena)).unwrap();
    let [doc, make, catalog, read] = slots.map(|slot| ids.row_id("provider.d.ts", slot).unwrap());
    let context = ProjectContext { programs: Some(vec![program("only", &[&files[0], &files[1]])]), ..Default::default() };
    let tree = Compilation::build_with_context(&files, &ids, Arc::clone(&arena), Some(&context), &HashSet::new());
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        assert_eq!(lookup.bound_import("main.ts", binding, false), Some(make));
        assert_eq!(lookup.bound_import_namespace("main.ts", binding), namespace);
        assert_eq!(targets(&tree.modules.programs, "only", "Catalog", true), Some(vec![catalog]));
        for function in [make, read] {
            let ty = lookup.return_type_id_of(function).unwrap();
            assert_eq!(super::super::head_decl::head_decl_id(tree.type_arena().unwrap(), ty), Some(doc));
            assert!(tree.type_arena().unwrap().accepts_nominal_context(ty, lookup.nominal_context()));
        }
    };
    check(&tree); tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new()); restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored); cold.ingest_from_db(db.conn()); check(&cold);
    }
}

#[test]
fn export_assignment_namespace_anchors_have_exact_program_declarations() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("provider.d.ts", "declare module 'provider' { namespace API { class Doc { touch(): void; } function make(): Doc; } export = API; }"),
        ("main.ts", "import API = require('provider'); API.make().touch();")]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let context = ProjectContext {
        programs: Some(vec![program("only", &[&files[0], &files[1]])]),
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &HashSet::new(),
    );
    let graph = files[1].flow.lexical.as_ref().unwrap();
    let root = graph
        .lookup(
            crate::indexer::lexical::ScopeId(0),
            graph.name_id("API").unwrap(),
        )
        .unwrap();
    let selected = tree.program_lookup("main.ts").unwrap();
    assert!(
        selected.bound_import_namespace("main.ts", root),
        "inputs={:#?}",
        tree.modules.inputs
    );
    let slot = files[0]
        .symbols
        .iter()
        .position(|s| s.name == "make")
        .unwrap();
    let expected = ids.row_id("provider.d.ts", slot).unwrap();
    let (&_, &binding) = graph
        .module
        .members
        .iter()
        .min_by_key(|(byte, _)| *byte)
        .unwrap();
    assert_eq!(
        selected.bound_import("main.ts", binding, false),
        Some(expected),
        "inputs={:#?}",
        tree.modules.inputs
    );
    let doc = ids
        .row_id(
            "provider.d.ts",
            files[0]
                .symbols
                .iter()
                .position(|s| s.name == "Doc")
                .unwrap(),
        )
        .unwrap();
    let returned = selected.return_type_id_of(expected).unwrap();
    assert_eq!(
        super::super::head_decl::head_decl_id(&arena, returned),
        Some(doc)
    );
}

fn inherited_touch_target(tree: &Compilation, caller: &ParsedFile, ids: &SymbolIds) -> Option<i64> {
    use super::super::{chain, file_lookup::FileLookup, testkit};
    let lookup = FileLookup::for_file(tree, caller, ids);
    let reference = caller
        .refs
        .iter()
        .find(|r| {
            r.kind == EdgeKind::Calls
                && r.chain
                    .as_ref()
                    .is_some_and(|c| c.segments.last().is_some_and(|s| s.name == "touch"))
        })
        .unwrap();
    let mut context = testkit::ref_ctx(
        reference,
        &caller.symbols[reference.source_symbol_index],
        vec![],
    );
    context.source_symbol_id = ids.row_id(&caller.path, reference.source_symbol_index);
    chain::bind_member_access(
        &context,
        &testkit::file_ctx(vec![], None),
        &lookup,
        &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
    )
    .ok()
    .map(|r| r.target_symbol_id)
}

#[test]
fn configured_base_applications_retarget_after_barrel_edits_and_provider_deletion() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[
        ("a.ts", "export class Doc { touch(): void {} } export class Parent<T> { next(): T { throw 0; } }"),
        ("b.ts", "export class Doc { touch(): void {} } export class Parent<T> { next(): T { throw 0; } }"),
        ("barrel.ts", "export { Parent, Doc } from './a';"),
        ("main.ts", "import { Parent, Doc } from './barrel'; class Child extends Parent<Doc> { run() { super.next().touch(); } }"),
    ]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let row = |file: usize, name: &str| {
        ids.row_id(
            &files[file].path,
            files[file]
                .symbols
                .iter()
                .position(|s| s.name == name)
                .unwrap(),
        )
        .unwrap()
    };
    let child = row(3, "Child");
    let context = |sources: Vec<&ParsedFile>| ProjectContext {
        programs: Some(vec![program("only", &sources)]),
        ..Default::default()
    };
    let initial = context(files.iter().collect());
    let mut tree = Compilation::build_with_context(
        &files[2..],
        &ids,
        Arc::clone(&arena),
        Some(&initial),
        &HashSet::new(),
    );
    let check = |tree: &Compilation, provider: Option<usize>| {
        let selected = tree.program_lookup("main.ts").unwrap();
        let parent = selected.parent_class_id(child);
        assert_eq!(parent, provider.map(|p| row(p, "Parent")));
        assert_eq!(
            inherited_touch_target(tree, &files[3], &ids),
            provider.map(|p| row(p, "touch"))
        );
        if let Some(provider) = provider {
            let arena = tree.type_arena().unwrap();
            let base = selected
                .canonical_type_info(child)
                .unwrap()
                .base_type_id
                .unwrap();
            assert!(arena.accepts_nominal_context(base, selected.nominal_context()));
            assert_eq!(super::super::head_decl::head_decl_id(arena, base), parent);
            let args = selected.parent_class_arg_ids_of(child, parent.unwrap());
            assert_eq!(args.len(), 1);
            assert_eq!(
                super::super::head_decl::head_decl_id(arena, args[0]),
                Some(row(provider, "Doc"))
            );
            let receiver = (&selected as &dyn SymbolLookup)
                .declaration_type(arena, child)
                .unwrap();
            let env = super::super::inherited_bindings::for_member(
                &selected,
                arena,
                receiver,
                Some(child),
                row(provider, "next"),
            )
            .unwrap();
            let returned = super::super::contract::generic_return::substitute(
                arena,
                selected.return_type_id_of(row(provider, "next")).unwrap(),
                &env,
            );
            assert_eq!(returned, args[0]);
        }
    };
    check(&tree, None);
    tree.ingest(&files[..2], &ids, &HashSet::new());
    check(&tree, Some(0));
    tree.persist_type_info(db.conn()).unwrap();
    let original_recipe = serde_json::to_value(
        &tree.modules.inputs["main.ts"]
            .globals
            .as_ref()
            .unwrap()
            .types
            .as_ref()
            .unwrap()
            .bases,
    )
    .unwrap();
    let changed = parse(
        &arena,
        &[("barrel.ts", "export { Parent, Doc } from './b';")],
    );
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let updated = context(vec![&files[0], &files[1], &changed[0], &files[3]]);
    let mut edited = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&updated),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, Some(1));
    assert_eq!(
        serde_json::to_value(
            &edited.modules.inputs["main.ts"]
                .globals
                .as_ref()
                .unwrap()
                .types
                .as_ref()
                .unwrap()
                .bases
        )
        .unwrap(),
        original_recipe
    );
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, Some(1));
    db.conn()
        .execute("DELETE FROM files WHERE path='b.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), restored);
    deleted.ingest_from_db(db.conn());
    check(&deleted, None);
}

#[test]
fn configured_uncalled_member_miss_cannot_retry_display_names() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("main.ts", "export {}; class Doc { touch() {} } class Parent { item!: Doc; } class Child extends Parent { run() { this.item.touch(); } }")]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let context = ProjectContext {
        programs: Some(vec![program("only", &[&files[0]])]),
        ..Default::default()
    };
    let tree =
        Compilation::build_with_context(&files, &ids, arena, Some(&context), &HashSet::new());
    assert!(inherited_touch_target(&tree, &files[0], &ids).is_some());
    let mut missing = files.into_iter().next().unwrap();
    let graph = missing.flow.lexical.as_mut().unwrap();
    let item = graph.name_id("item").unwrap();
    let selectors = &mut graph.globals.as_mut().unwrap().selectors;
    let before = selectors.len();
    selectors.retain(|_, name| *name != item);
    assert!(selectors.len() < before);
    assert_eq!(
        inherited_touch_target(&tree, &missing, &ids),
        None,
        "a missing source selector cannot fall back to the unchanged segment display name"
    );
}

#[test]
fn same_source_base_has_program_owned_parent_and_argument_identities() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[
        ("shared.ts", "import { Parent } from 'provider'; export class Doc { touch(): void {} } export class Child extends Parent<Doc> {}"),
        ("a.d.ts", "declare module 'provider' { class Parent<T> { next(): T; } }"),
        ("b.d.ts", "declare module 'provider' { class Parent<T> { next(): T; } }"),
        ("a.ts", "import { Child } from './shared'; function run(child: Child) { child.next().touch(); }"),
        ("b.ts", "import { Child } from './shared'; function run(child: Child) { child.next().touch(); }"),
    ]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let row = |file: usize, name: &str| {
        ids.row_id(
            &files[file].path,
            files[file]
                .symbols
                .iter()
                .position(|s| s.name == name)
                .unwrap(),
        )
        .unwrap()
    };
    let context = ProjectContext {
        programs: Some(vec![
            program("a", &[&files[0], &files[1], &files[3]]),
            program("b", &[&files[0], &files[2], &files[4]]),
        ]),
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let mut arguments = Vec::new();
        for (path, provider, consumer) in [("a.ts", 1, 3), ("b.ts", 2, 4)] {
            let lookup = tree.program_lookup(path).unwrap();
            let parent = row(provider, "Parent");
            assert_eq!(lookup.parent_class_ids(row(0, "Child")), [parent]);
            let args = lookup.parent_class_arg_ids_of(row(0, "Child"), parent);
            assert_eq!(args.len(), 1);
            arguments.push(args[0]);
            assert_eq!(
                super::super::head_decl::head_decl_id(tree.type_arena().unwrap(), args[0]),
                Some(row(0, "Doc"))
            );
            assert!(tree
                .type_arena()
                .unwrap()
                .accepts_nominal_context(args[0], lookup.nominal_context()));
            assert_eq!(
                inherited_touch_target(tree, &files[consumer], &ids),
                Some(row(0, "touch"))
            );
        }
        assert_ne!(arguments[0], arguments[1]);
        assert!(tree
            .program_lookup("shared.ts")
            .unwrap()
            .parent_class_ids(row(0, "Child"))
            .is_empty());
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
fn configured_private_selectors_reject_out_of_scope_or_wrong_receiver_brands() {
    for source in [
        "export {}; class Doc { touch() {} } class Parent { #item!: Doc; } class Child extends Parent { run() { this.#item.touch(); } }",
        "export {}; class Doc { touch() {} } class Other { #item!: Doc; } class Parent { #item!: Doc; run(value: Other) { value.#item.touch(); } }",
    ] {
        let arena = Arc::new(TypeArena::new());
        let files = parse(&arena, &[("main.ts", source)]);
        assert!(files[0].symbols.iter().any(|s| s.name == "#item"));
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(&db, &files, "internal", Some(&arena)).unwrap();
        let context = ProjectContext { programs: Some(vec![program("only", &[&files[0]])]), ..Default::default() };
        let tree = Compilation::build_with_context(&files, &ids, Arc::clone(&arena), Some(&context), &HashSet::new());
        assert_eq!(inherited_touch_target(&tree, &files[0], &ids), None, "{source}");
        tree.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new()); restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &SymbolIds::default(), restored); cold.ingest_from_db(db.conn());
        assert_eq!(inherited_touch_target(&cold, &files[0], &ids), None, "cold: {source}");
    }
}

#[test]
fn configured_base_applications_survive_filtered_portable_sources() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    use crate::type_checker::core::types::GenericParamData;
    let original = TypeArena::new();
    let mut providers = parse(&original, &[("provider.ts",
        "export function discard() { class Parent { wrong() {} } } export class Doc { touch() {} } export class Ancestor<T> { next(): T { throw 0; } } export class Parent<U> extends Ancestor<U> {}")]);
    let before = providers[0].symbols.len();
    reduce_to_contract(&mut providers[0]);
    assert!(providers[0].symbols.len() < before);
    let payload =
        serde_json::to_string(&CachedParse::from_parsed(&providers[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    arena.intern_generic(GenericParamData {
        name: "noise".into(),
        owner_symbol_index: 999,
        bound: None,
        kind: Default::default(),
    });
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut provider = cached.into_parsed(
        &arena,
        "provider.ts",
        &providers[0].content_hash,
        providers[0].size,
        None,
    );
    provider.content = providers[0].content.clone();
    reduce_to_contract(&mut provider);
    assert_eq!(
        provider
            .symbols
            .iter()
            .filter(|s| s.name == "Parent")
            .count(),
        1
    );
    let mut files = vec![provider];
    files.extend(parse(&arena, &[("use.ts", "import { Parent, Doc } from './provider'; class Child extends Parent<Doc> { run() { super.next().touch(); } }")]));
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let row = |file: usize, name: &str| {
        ids.row_id(
            &files[file].path,
            files[file]
                .symbols
                .iter()
                .position(|s| s.name == name)
                .unwrap(),
        )
        .unwrap()
    };
    let context = ProjectContext {
        programs: Some(vec![program("only", &[&files[0], &files[1]])]),
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("use.ts").unwrap();
        assert_eq!(
            lookup.parent_class_id(row(1, "Child")),
            Some(row(0, "Parent"))
        );
        assert_eq!(
            lookup.parent_class_id(row(0, "Parent")),
            Some(row(0, "Ancestor"))
        );
        let args = lookup.parent_class_arg_ids_of(row(0, "Parent"), row(0, "Ancestor"));
        assert_eq!(
            args,
            &[tree.type_arena().unwrap().generic_type(
                lookup
                    .canonical_type_info(row(0, "Parent"))
                    .unwrap()
                    .generic_param_ids[0]
            )]
        );
        assert_eq!(
            inherited_touch_target(tree, &files[1], &ids),
            Some(row(0, "touch"))
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
fn configured_member_surfaces_survive_filtered_portable_and_cold_sources() {
    use crate::indexer::lexical::globals::member_surface::{Key, Kind, Root};
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let original = TypeArena::new();
    let mut providers = parse(&original, &[("provider.ts", "function discard() { class Hidden { wrong() {} } } declare const Keys: any; interface Catalog<T> { (item: T): T; new<U>(item: U): Catalog<U>; [index: number]: T; [Keys.iterator](): T; read(): T; read(value: T): T; }")]);
    let before = providers[0].symbols.len();
    reduce_to_contract(&mut providers[0]);
    assert!(
        providers[0].symbols.len() < before,
        "must actually filter source row slots"
    );
    let payload =
        serde_json::to_string(&CachedParse::from_parsed(&providers[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut provider = cached.into_parsed(
        &arena,
        "provider.ts",
        &providers[0].content_hash,
        providers[0].size,
        None,
    );
    provider.content = providers[0].content.clone();
    reduce_to_contract(&mut provider);
    let mut files = vec![provider];
    files.extend(parse(
        &arena,
        &[("augment.d.ts", "interface Catalog<T> { other(): T; }")],
    ));
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "external",
        Some(&arena),
    )
    .unwrap();
    let context = ProjectContext {
        programs: Some(vec![program("only", &[&files[0], &files[1]])]),
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &HashSet::new(),
    );
    let surface = |tree: &Compilation| {
        tree.modules.inputs["provider.ts"]
            .globals
            .as_ref()
            .unwrap()
            .roots
            .iter()
            .find(|part| part.kind == SymbolKind::Interface)
            .unwrap()
            .surface
            .clone()
            .unwrap()
    };
    let expected = surface(&tree);
    assert_eq!(expected.len(), 6);
    assert_eq!(
        expected.iter().map(|m| m.kind).collect::<Vec<_>>(),
        [
            Kind::Call,
            Kind::Construct,
            Kind::Index,
            Kind::Method,
            Kind::Method,
            Kind::Method
        ]
    );
    assert!(matches!(
        &expected[3].key,
        Key::Computed {
            root: Root::Binding(_),
            ..
        }
    ));
    assert!(
        expected[4].slot.is_some() && expected[5].slot.is_some(),
        "positive physical row evidence required"
    );
    assert_ne!(
        expected[4].slot, expected[5].slot,
        "overloads cannot share one name-correlated row"
    );
    assert!(
        targets(&tree.modules.programs, "only", "Catalog", true).is_none(),
        "syntax alone must not relax merge guards"
    );
    let lowered = super::super::program_input::capture(&files[0], &ids).unwrap();
    for symbol in &mut files[0].symbols {
        symbol.name = "poison".into();
        symbol.qualified_name = "poison.display".into();
        symbol.signature = None;
    }
    assert_eq!(
        serde_json::to_value(super::super::program_input::capture(&files[0], &ids).unwrap())
            .unwrap(),
        serde_json::to_value(lowered).unwrap(),
        "display fields are not member identities"
    );
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    assert_eq!(surface(&cold), expected);
    assert!(targets(&cold.modules.programs, "only", "Catalog", true).is_none());
}

#[test]
fn duplicate_index_domains_do_not_merge_by_parameter_spelling() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/member_index_legality_fixtures.json"
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
            "external",
            Some(&arena),
        )
        .unwrap();
        let context = ProjectContext {
            programs: Some(vec![program("only", &files.iter().collect::<Vec<_>>())]),
            ..Default::default()
        };
        let tree = Compilation::build_with_context(
            &files,
            &ids,
            Arc::clone(&arena),
            Some(&context),
            &HashSet::new(),
        );
        assert!(targets(&tree.modules.programs, "only", "Catalog", true).is_none());
        tree.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
        cold.ingest_from_db(db.conn());
        assert!(targets(&cold.modules.programs, "only", "Catalog", true).is_none());
        // The legal control also stays incomplete until index compatibility is
        // materialized. This test must not claim general index-signature support.
    }
}

#[test]
fn rowless_signature_types_survive_filtered_portable_and_cold_sources() {
    use super::super::program_types::source_signatures::SignatureId;
    use crate::indexer::lexical::globals::member_surface::Kind;
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    use crate::type_checker::core::types::Type;
    let original = TypeArena::new();
    let source = "function discard() { class Hidden { wrong() {} } } interface Catalog<T> { <U extends T = T>(item: U): U; new<U extends T>(item: U): Catalog<U>; read<V>(callback: <W>(item: W) => T, optional?: V, ...rest: V[]): V; }";
    let mut providers = parse(&original, &[("provider.ts", source)]);
    // Exercise a provider with no unnamed signature navigation rows. Route
    // those rows through the normal filter/remapping path; source is unchanged.
    let graph = providers[0].flow.lexical.as_ref().unwrap();
    let hidden_rows: Vec<_> = graph
        .globals
        .as_ref()
        .unwrap()
        .roots
        .iter()
        .filter_map(|p| p.surface.as_ref())
        .flatten()
        .filter(|m| matches!(m.kind, Kind::Call | Kind::Construct))
        .filter_map(|m| m.slot)
        .collect();
    assert_eq!(hidden_rows.len(), 2);
    let discarded = providers[0]
        .symbols
        .iter()
        .position(|s| s.byte_offset == 0)
        .unwrap();
    for row in hidden_rows {
        providers[0].symbols[row].parent_index = Some(discarded);
    }
    let before = providers[0].symbols.len();
    reduce_to_contract(&mut providers[0]);
    assert!(providers[0].symbols.len() < before);
    let payload =
        serde_json::to_string(&CachedParse::from_parsed(&providers[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut file = cached.into_parsed(
        &arena,
        "provider.ts",
        &providers[0].content_hash,
        providers[0].size,
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
    let context = ProjectContext {
        programs: Some(vec![program("only", &[&file])]),
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &HashSet::new(),
    );
    let input = tree.modules.inputs["provider.ts"].globals.as_ref().unwrap();
    let part = input
        .roots
        .iter()
        .find(|p| p.kind == SymbolKind::Interface)
        .unwrap();
    let owner = part.declaration.unwrap();
    let surface = part.surface.as_ref().unwrap();
    assert_eq!(
        surface.iter().map(|m| m.kind).collect::<Vec<_>>(),
        [Kind::Call, Kind::Construct, Kind::Method]
    );
    let sites: Vec<_> = surface.iter().map(|m| SignatureId(m.span)).collect();
    assert!(
        surface[..2].iter().all(|m| m.slot.is_none()),
        "test must exercise rowless owners"
    );
    let typed = input.types.as_ref().unwrap();
    let nested = typed
        .source_signatures
        .iter()
        .find(|s| s.id.0.start == source.find("<W>").unwrap() as u32)
        .unwrap()
        .id;
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("provider.ts").unwrap();
        let arena = tree.type_arena().unwrap();
        let outer =
            arena.generic_type(lookup.canonical_type_info(owner).unwrap().generic_param_ids[0]);
        let call = lookup.signature(sites[0]).unwrap();
        let construct = lookup.signature(sites[1]).unwrap();
        let method = lookup.signature(sites[2]).unwrap();
        let callback = lookup.signature(nested).unwrap();
        let u = arena.generic_type(call.generic_parameters[0]);
        assert_eq!(call.parameters, vec![u]);
        assert_eq!(call.result, Some(u));
        assert_eq!(call.constraints, vec![Some(outer)]);
        assert_eq!(call.defaults, vec![Some(outer)]);
        assert_ne!(call.generic_parameters, construct.generic_parameters);
        assert_eq!(
            construct.parameters,
            vec![arena.generic_type(construct.generic_parameters[0])]
        );
        assert_eq!(construct.constraints, vec![Some(outer)]);
        let Type::Apply { base, args } = arena.get(construct.result.unwrap()) else {
            panic!("constructor result application");
        };
        assert_eq!(
            (&lookup as &dyn SymbolLookup).declaration_type(arena, owner),
            Some(base)
        );
        assert_eq!(args, construct.parameters);
        assert_eq!(
            callback.parameters,
            vec![arena.generic_type(callback.generic_parameters[0])]
        );
        assert_eq!(callback.result, Some(outer));
        let Type::Callable(nested_type) = arena.get(method.parameters[0]) else {
            panic!("source callable identity");
        };
        assert_eq!(
            nested_type
                .parameters
                .iter()
                .map(|p| p.ty)
                .collect::<Vec<_>>(),
            callback.parameters
        );
        assert_eq!(nested_type.result, outer);
        assert_eq!(nested_type.origin.signature, nested.0);
        assert_eq!(
            nested_type.generics[0].parameter,
            arena.generic_type(callback.generic_parameters[0])
        );
        assert!(method.syntax.parameters[1].optional && method.syntax.parameters[2].rest);
        let row = surface[2].slot.unwrap();
        assert_eq!(
            method.generic_parameters,
            lookup.canonical_type_info(row).unwrap().generic_param_ids
        );
        assert_eq!(
            lookup.source_signature_parameter(crate::types::SourceSpan { start: 0, end: 1 }, 0),
            None
        );
    };
    check(&tree);
    let expected = serde_json::to_value(typed).unwrap();
    for symbol in &mut file.symbols {
        symbol.name = "poison".into();
        symbol.qualified_name = "poison.display".into();
        symbol.signature = None;
    }
    assert_eq!(
        serde_json::to_value(super::super::program_types::capture(
            &file, &ids, &tree, &arena
        ))
        .unwrap(),
        expected
    );
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
    assert_eq!(
        serde_json::to_value(
            cold.modules.inputs["provider.ts"]
                .globals
                .as_ref()
                .unwrap()
                .types
                .as_ref()
                .unwrap()
        )
        .unwrap(),
        expected
    );
    file.content_hash = "stale-source".into();
    let stale = super::super::file_lookup::FileLookup::for_file(&cold, &file, &ids);
    assert_eq!(stale.source_signature_parameter(sites[0].0, 0), None);
}

#[test]
fn atomic_signature_values_survive_filtered_portable_and_cold_sources() {
    use crate::indexer::lexical::type_syntax::TypeExpr;
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    use crate::type_checker::core::types::{Intrinsic, Type};
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/atomic_type_fixtures.json"
    ))
    .unwrap();
    let mut source = String::from("interface Values { ");
    for (i, case) in cases.iter().enumerate() {
        source.push_str(&format!("v{i}: {}; ", case["syntax"].as_str().unwrap()));
    }
    source.push_str("missing: Missing; absent; method<T extends string = 'x'>(value: T): T; }");
    let original = TypeArena::new();
    let mut providers = parse(&original, &[("provider.d.ts", &source)]);
    reduce_to_contract(&mut providers[0]);
    let graph = providers[0].flow.lexical.as_ref().unwrap();
    let expected: Vec<_> = graph.types.signatures[..cases.len()]
        .iter()
        .map(|s| {
            let ty = match s.result.as_ref().unwrap() {
                TypeExpr::Intrinsic(kind) => Type::Intrinsic(*kind),
                TypeExpr::Literal(value) => Type::Literal(value.clone()),
                other => panic!("atomic source recipe lost: {other:?}"),
            };
            (s.id, s.declaration.unwrap(), ty)
        })
        .collect();
    let missing = graph.types.signatures[cases.len()].id;
    let absent = graph.types.signatures[cases.len() + 1].id;
    let method = graph.types.signatures[cases.len() + 2].id;
    let payload =
        serde_json::to_string(&CachedParse::from_parsed(&providers[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    arena.class("poison");
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut file = cached.into_parsed(
        &arena,
        "provider.d.ts",
        &providers[0].content_hash,
        providers[0].size,
        None,
    );
    file.content = Some(source);
    reduce_to_contract(&mut file);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&file),
        "external",
        Some(&arena),
    )
    .unwrap();
    let context = ProjectContext {
        programs: Some(vec![program("only", &[&file])]),
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("provider.d.ts").unwrap();
        let arena = tree.type_arena().unwrap();
        for (site, slot, ty) in &expected {
            let result = lookup.signature(*site).unwrap().result.unwrap();
            assert_eq!(&arena.get(result), ty);
            assert_eq!(result, arena.intern(ty.clone()), "canonical value identity");
            let row = ids.row_id("provider.d.ts", *slot).unwrap();
            assert_eq!(
                lookup.canonical_type_info(row).unwrap().field_type_id,
                Some(result)
            );
            assert_ne!(result, arena.intern(Type::Unknown));
        }
        assert_eq!(
            arena.get(lookup.signature(missing).unwrap().result.unwrap()),
            Type::Unknown
        );
        assert_eq!(lookup.signature(absent).unwrap().result, None);
        let method = lookup.signature(method).unwrap();
        assert_eq!(
            method.constraints,
            vec![Some(arena.intern(Type::Intrinsic(Intrinsic::String)))]
        );
        assert_eq!(
            method.defaults,
            vec![Some(arena.intern(Type::Literal(
                crate::type_checker::core::types::LitValue::Str("x".into())
            )))]
        );
        assert_eq!(
            method.result,
            Some(arena.generic_type(method.generic_parameters[0]))
        );
        assert_eq!(method.parameters, vec![method.result.unwrap()]);
    };
    check(&tree);
    let expected_payload = serde_json::to_value(super::super::program_types::capture(
        &file, &ids, &tree, &arena,
    ))
    .unwrap();
    for symbol in &mut file.symbols {
        symbol.name = "poison".into();
        symbol.qualified_name = "poison".into();
        symbol.signature = None;
    }
    assert_eq!(
        serde_json::to_value(super::super::program_types::capture(
            &file, &ids, &tree, &arena
        ))
        .unwrap(),
        expected_payload
    );
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
}

#[test]
fn configured_type_operators_preserve_source_generic_ids_through_portable_and_cold_inputs() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    use crate::type_checker::core::types::{Intrinsic, LitValue, Type, TypeId, TypeOperator as Op};
    use serde_json::Value;
    fn expected(shape: &Value, arena: &TypeArena, params: &[TypeId], array: TypeId) -> TypeId {
        let child = |i: usize| expected(&shape[i], arena, params, array);
        arena.intern(match shape[0].as_str().unwrap() {
            "param" => return params[shape[1].as_u64().unwrap() as usize],
            "keyof" => Type::Operator(Op::KeyOf(child(1))),
            "readonly" => Type::Operator(Op::Readonly(child(1))),
            "index" => Type::Operator(Op::IndexedAccess {
                object: child(1),
                index: child(2),
            }),
            "conditional" => Type::Operator(Op::Conditional {
                check: child(2),
                extends: child(3),
                when_true: child(4),
                when_false: child(5),
                distributive: shape[1].as_bool(),
            }),
            "tuple" => Type::Tuple((1..shape.as_array().unwrap().len()).map(child).collect()),
            "array" => Type::Apply {
                base: array,
                args: vec![child(1)],
            },
            "intrinsic" => Type::Intrinsic(match shape[1].as_str().unwrap() {
                "string" => Intrinsic::String,
                "number" => Intrinsic::Number,
                "never" => Intrinsic::Never,
                _ => panic!(),
            }),
            "number" => Type::Literal(LitValue::Number(shape[1].as_f64().unwrap().to_bits())),
            _ => panic!("{shape:?}"),
        })
    }
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/operator_type_fixtures.json"
    ))
    .unwrap();
    let mut source =
        String::from("interface Array<T> { item: T; } interface Ops<T, K extends keyof T> { ");
    for (i, case) in cases.iter().enumerate() {
        source.push_str(&format!("v{i}: {}; ", case["syntax"].as_str().unwrap()));
    }
    source.push_str("missing: Missing[keyof Missing]; }");
    let original = TypeArena::new();
    let mut providers = parse(&original, &[("provider.d.ts", &source)]);
    reduce_to_contract(&mut providers[0]);
    let payload =
        serde_json::to_string(&CachedParse::from_parsed(&providers[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    arena.class("poison");
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut file = cached.into_parsed(
        &arena,
        "provider.d.ts",
        &providers[0].content_hash,
        providers[0].size,
        None,
    );
    file.content = Some(source.clone());
    reduce_to_contract(&mut file);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&file),
        "external",
        Some(&arena),
    )
    .unwrap();
    let context = ProjectContext {
        programs: Some(vec![program("only", &[&file])]),
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &HashSet::new(),
    );
    let owner = file.symbols.iter().position(|s| s.name == "Ops").unwrap();
    let array = ids
        .row_id(
            "provider.d.ts",
            file.symbols.iter().position(|s| s.name == "Array").unwrap(),
        )
        .unwrap();
    let graph = file.flow.lexical.as_ref().unwrap();
    let signatures: Vec<_> = (0..cases.len())
        .map(|i| {
            graph
                .types
                .signatures
                .iter()
                .find(|s| s.id.0.start == source.find(&format!("v{i}:")).unwrap() as u32)
                .unwrap()
                .id
        })
        .collect();
    let missing = graph
        .types
        .signatures
        .iter()
        .find(|s| s.id.0.start == source.find("missing:").unwrap() as u32)
        .unwrap()
        .id;
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("provider.d.ts").unwrap();
        let arena = tree.type_arena().unwrap();
        let params: Vec<_> = lookup
            .canonical_type_info(ids.row_id("provider.d.ts", owner).unwrap())
            .unwrap()
            .generic_param_ids
            .iter()
            .map(|&p| arena.generic_type(p))
            .collect();
        assert_eq!(params.len(), 2);
        let array = (&lookup as &dyn SymbolLookup)
            .declaration_type(arena, array)
            .unwrap();
        for (case, &site) in cases.iter().zip(&signatures) {
            let actual = lookup.signature(site).unwrap().result.unwrap();
            assert_eq!(
                actual,
                expected(&case["shape"], arena, &params, array),
                "{}",
                case["syntax"]
            );
        }
        let unknown = arena.intern(Type::Unknown);
        let keys = arena.intern(Type::Operator(Op::KeyOf(unknown)));
        assert_eq!(
            lookup.signature(missing).unwrap().result,
            Some(arena.intern(Type::Operator(Op::IndexedAccess {
                object: unknown,
                index: keys
            })))
        );
    };
    check(&tree);
    let expected_payload = serde_json::to_value(super::super::program_types::capture(
        &file, &ids, &tree, &arena,
    ))
    .unwrap();
    for symbol in &mut file.symbols {
        symbol.name = "poison".into();
        symbol.qualified_name = "poison".into();
        symbol.signature = None;
    }
    assert_eq!(
        serde_json::to_value(super::super::program_types::capture(
            &file, &ids, &tree, &arena
        ))
        .unwrap(),
        expected_payload
    );
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
}

#[test]
fn imported_operator_operands_retarget_after_provider_edits_without_recapturing_consumer() {
    use crate::type_checker::core::types::{Type, TypeOperator};
    for configured in [false, true] {
        let arena = Arc::new(TypeArena::new());
        let source =
            "import { Doc } from './barrel'; export function read(): keyof Doc { throw 0; }";
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
        let context = |sources: Vec<&ParsedFile>| ProjectContext {
            programs: configured.then(|| vec![program("only", &sources)]),
            ..Default::default()
        };
        let tree = Compilation::build_with_context(
            &files,
            &ids,
            Arc::clone(&arena),
            Some(&context(files.iter().collect())),
            &HashSet::new(),
        );
        let function = ids
            .row_id(
                "main.ts",
                files[3]
                    .symbols
                    .iter()
                    .position(|s| s.name == "read")
                    .unwrap(),
            )
            .unwrap();
        let check = |tree: &Compilation, expected: usize| {
            let lookup = super::super::file_lookup::FileLookup::for_file(tree, &files[3], &ids);
            let arena = tree.type_arena().unwrap();
            let result = lookup.return_type_id_of(function).unwrap();
            let Type::Operator(TypeOperator::KeyOf(operand)) = arena.get(result) else {
                panic!("{:?}", arena.get(result))
            };
            let Type::Decl { symbol_id, .. } = arena.get(operand) else {
                panic!("{:?}", arena.get(operand))
            };
            assert_eq!(Some(symbol_id), ids.row_id(&files[expected].path, 0));
        };
        check(&tree, 0);
        tree.persist_type_info(db.conn()).unwrap();
        let changed = parse(&arena, &[("barrel.ts", "export { Doc } from './right';")]);
        let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &changed,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let updated = context(vec![&files[0], &files[1], &changed[0], &files[3]]);
        let mut edited = Compilation::build_with_context(
            &changed,
            &changed_ids,
            Arc::clone(&arena),
            Some(&updated),
            &HashSet::new(),
        );
        edited.ingest_from_db(db.conn());
        check(&edited, 1);
        edited.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
        cold.ingest_from_db(db.conn());
        check(&cold, 1);
        db.conn()
            .execute("DELETE FROM files WHERE path='right.ts'", [])
            .unwrap();
        let mut deleted = Compilation::build(&[], &SymbolIds::default(), restored);
        deleted.ingest_from_db(db.conn());
        let lookup = super::super::file_lookup::FileLookup::for_file(&deleted, &files[3], &ids);
        if let Some(result) = lookup.return_type_id_of(function) {
            match deleted.type_arena().unwrap().get(result) {
                Type::Unknown => {}
                Type::Operator(TypeOperator::KeyOf(inner)) => {
                    assert_eq!(deleted.type_arena().unwrap().get(inner), Type::Unknown)
                }
                other => panic!("deleted provider revived an operator operand: {other:?}"),
            }
        }
    }
}

#[test]
fn atomic_signature_edits_rebind_unchanged_consumer_types_without_display_names() {
    use crate::type_checker::core::types::{Intrinsic, Type};
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("provider.ts", "export function read(): string { return ''; }"),
        ("main.ts", "import { read } from './provider'; export function run() { const result = read(); return result; }")]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let context = |sources: Vec<&ParsedFile>| ProjectContext {
        programs: Some(vec![program("only", &sources)]),
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(files.iter().collect())),
        &HashSet::new(),
    );
    let read_byte = files[1].content.as_ref().unwrap().find("read();").unwrap() as u32;
    let check = |tree: &Compilation, expected: Intrinsic| {
        let lookup = super::super::file_lookup::FileLookup::for_file(tree, &files[1], &ids);
        lookup.set_cursor(read_byte);
        let target = lookup
            .local_reference(read_byte)
            .unwrap()
            .declaration
            .unwrap();
        let actual = lookup.return_type_id_of(target).unwrap();
        assert_eq!(
            tree.type_arena().unwrap().get(actual),
            Type::Intrinsic(expected)
        );
    };
    check(&tree, Intrinsic::String);
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(
        &arena,
        &[(
            "provider.ts",
            "export function read(): number { return 1; }",
        )],
    );
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let updated = context(vec![&changed[0], &files[1]]);
    let mut edited = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&updated),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, Intrinsic::Number);
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold, Intrinsic::Number);
}

#[test]
fn intrinsic_receivers_use_global_wrapper_ids_without_changing_source_types() {
    use crate::type_checker::core::types::{Intrinsic, LitValue, Type};
    for configured in [false, true] {
        for supplied in [false, true] {
            let arena = Arc::new(TypeArena::new());
            let files = parse(&arena, &[("lib.d.ts", if supplied { "interface String { replace(): string; }" } else { "interface Unrelated {}" }),
                ("main.ts", "export {}; interface String { wrong(): void; } function run(text: string) { text.replace(); }"),
                ("other.ts", "export interface String { replace(): number; }")]);
            let db = crate::Database::open_in_memory().unwrap();
            let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
                &db,
                &files,
                "internal",
                Some(&arena),
            )
            .unwrap();
            let context = ProjectContext {
                programs: configured
                    .then(|| vec![program("only", &files.iter().collect::<Vec<_>>())]),
                ..Default::default()
            };
            let tree = Compilation::build_with_context(
                &files,
                &ids,
                Arc::clone(&arena),
                Some(&context),
                &HashSet::new(),
            );
            let check = |tree: &Compilation| {
                let lookup = super::super::file_lookup::FileLookup::for_file(tree, &files[1], &ids);
                let arena = tree.type_arena().unwrap();
                let ty = arena.intern(Type::Intrinsic(Intrinsic::String));
                let projected = super::super::program_view::project_intrinsic(&lookup, arena, ty);
                let literal = arena.intern(Type::Literal(LitValue::Str("exact".into())));
                assert_eq!(
                    super::super::program_view::project_intrinsic(&lookup, arena, literal),
                    projected
                );
                assert_eq!(arena.get(ty), Type::Intrinsic(Intrinsic::String));
                assert_eq!(
                    arena.get(literal),
                    Type::Literal(LitValue::Str("exact".into()))
                );
                for kind in [
                    Intrinsic::Unknown,
                    Intrinsic::Any,
                    Intrinsic::Undefined,
                    Intrinsic::Null,
                    Intrinsic::Never,
                ] {
                    assert_eq!(lookup.intrinsic_member_type(kind), None);
                }
                if supplied {
                    assert_eq!(
                        super::super::head_decl::head_decl_id(arena, projected.unwrap()),
                        ids.row_id("lib.d.ts", 0)
                    );
                    if configured {
                        let selector = files[1]
                            .content
                            .as_ref()
                            .unwrap()
                            .find("replace();")
                            .unwrap() as u32;
                        let bound = lookup.bound_method(ty, selector).unwrap().unwrap();
                        assert_eq!(Some(bound.declaration), ids.row_id("lib.d.ts", 1));
                    }
                } else {
                    assert_eq!(
                        projected, None,
                        "module/local namesakes must not supply a primitive wrapper"
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
}

#[test]
fn source_signature_ids_are_scoped_by_configured_source_and_program() {
    use super::super::program_types::source_signatures::SignatureId;
    let source = "export interface Catalog<T> { read(callback: <U>(v: U) => T): T; }";
    let arena = Arc::new(TypeArena::new());
    let files = parse(
        &arena,
        &[("a.ts", source), ("b.ts", source), ("shared.ts", source)],
    );
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let context = ProjectContext {
        programs: Some(vec![
            program("a", &[&files[0], &files[2]]),
            program("b", &[&files[1], &files[2]]),
        ]),
        ..Default::default()
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &HashSet::new(),
    );
    let site = files[0]
        .flow
        .lexical
        .as_ref()
        .unwrap()
        .types
        .signatures
        .iter()
        .find(|s| s.id.0.start == source.find("<U>").unwrap() as u32)
        .unwrap();
    assert!(site.declaration.is_none());
    let site: SignatureId = site.id;
    let check = |tree: &Compilation| {
        let a = tree.program_lookup("a.ts").unwrap();
        let b = tree.program_lookup("b.ts").unwrap();
        let a = a.signature(site).unwrap();
        let b = b.signature(site).unwrap();
        assert_eq!(a.generic_parameters.len(), 1);
        assert_eq!(b.generic_parameters.len(), 1);
        assert_ne!(a.generic_parameters, b.generic_parameters);
        assert_ne!(a.parameters, b.parameters);
        assert_ne!(
            a.result, b.result,
            "outer generic owner also belongs to its program"
        );
        assert!(
            tree.program_lookup("shared.ts")
                .unwrap()
                .signature(site)
                .is_none(),
            "ambiguous configured source cannot select an arena"
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
fn source_signature_constraints_retarget_after_import_edits_and_deletion() {
    use crate::type_checker::core::types::Type;
    let arena = Arc::new(TypeArena::new());
    let source = "import { Doc } from './barrel'; export interface Factory { <T extends Doc = Doc>(value: T): T; }";
    let files = parse(
        &arena,
        &[
            ("left.ts", "export class Doc {}"),
            ("right.ts", "export class Doc {}"),
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
    let context = |sources: Vec<&ParsedFile>| ProjectContext {
        programs: Some(vec![program("only", &sources)]),
        ..Default::default()
    };
    let mut tree = Compilation::build_with_context(
        &files[3..],
        &ids,
        Arc::clone(&arena),
        Some(&context(files.iter().collect())),
        &HashSet::new(),
    );
    let site = files[3]
        .flow
        .lexical
        .as_ref()
        .unwrap()
        .types
        .signatures
        .iter()
        .find(|s| s.id.0.start == source.find("<T").unwrap() as u32)
        .unwrap()
        .id;
    assert!(
        tree.program_lookup("main.ts")
            .unwrap()
            .signature(site)
            .is_none(),
        "missing source providers remain authoritative"
    );
    tree.ingest(&files[..3], &ids, &HashSet::new());
    let check = |tree: &Compilation, expected: Option<usize>| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        let Some(expected) = expected else {
            assert!(lookup.signature(site).is_none());
            return;
        };
        let bound = lookup.signature(site).unwrap();
        let row = ids.row_id(&files[expected].path, 0).unwrap();
        let ty = (&lookup as &dyn SymbolLookup)
            .declaration_type(tree.type_arena().unwrap(), row)
            .unwrap();
        assert_eq!(bound.constraints, vec![Some(ty)]);
        assert_eq!(bound.defaults, vec![Some(ty)]);
        assert!(matches!(
            tree.type_arena().unwrap().get(bound.result.unwrap()),
            Type::Generic { .. }
        ));
    };
    check(&tree, Some(0));
    tree.persist_type_info(db.conn()).unwrap();
    let original = serde_json::to_value(
        &tree.modules.inputs["main.ts"]
            .globals
            .as_ref()
            .unwrap()
            .types
            .as_ref()
            .unwrap()
            .source_signatures,
    )
    .unwrap();
    let changed = parse(&arena, &[("barrel.ts", "export { Doc } from './right';")]);
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let updated = context(vec![&files[0], &files[1], &changed[0], &files[3]]);
    let mut edited = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&updated),
        &HashSet::new(),
    );
    edited.ingest_from_db(db.conn());
    check(&edited, Some(1));
    assert_eq!(
        serde_json::to_value(
            &edited.modules.inputs["main.ts"]
                .globals
                .as_ref()
                .unwrap()
                .types
                .as_ref()
                .unwrap()
                .source_signatures
        )
        .unwrap(),
        original
    );
    edited.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, Some(1));
    db.conn()
        .execute("DELETE FROM files WHERE path='right.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), restored);
    deleted.ingest_from_db(db.conn());
    check(&deleted, None);
}

#[test]
#[ignore = "Read-only configured-program signature provenance for the pinned real compiler cohort"]
fn diagnose_pinned_program_global_and_signature_barriers() {
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
    let mut context = crate::indexer::project_context::build_project_context(&manifest.root);
    context.programs = Some(vec![Program {
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
        Some(&context),
        &HashSet::new(),
    );
    let graph = &tree.modules.programs;
    let program = graph.program("diagnostic").unwrap();
    assert!(
        graph.nominal_context(program).is_some(),
        "source capture remains incomplete"
    );
    let selected = tree.program_lookup(&files[0].path).unwrap();
    for spelling in [
        "Array",
        "ReadonlyArray",
        "Set",
        "Map",
        "Promise",
        "PromiseConstructor",
    ] {
        let name = graph.name(program, spelling).unwrap();
        let parts: Vec<_> = tree.modules.inputs.values().filter_map(|input| input.globals.as_ref().map(|globals| (input, globals)))
            .flat_map(|(input, globals)| globals.roots.iter().chain(&globals.augmentations).filter(move |part| part.name == spelling)
                .map(move |part| serde_json::json!({"file":input.path,"row":part.declaration,"type":part.type_space,
                    "plain_parameters":part.plain_parameters,"plain_header":part.plain_header,"plain_merge":part.plain_merge,"members":part.members.len()}))).collect();
        let admitted: Vec<_> = parts
            .iter()
            .filter_map(|p| p["row"].as_i64())
            .filter(|&row| selected.symbol_by_id(row).is_some())
            .collect();
        println!(
            "{}",
            serde_json::json!({"global":spelling,"value":format!("{:?}",graph.global(program,name,false)),
            "type":format!("{:?}",graph.global(program,name,true)),"selected_admitted_rows":admitted,"parts":parts})
        );
    }
    for file in files.iter().filter(|file| {
        file.path.ends_with("/subscribable.ts") || file.path.ends_with("/focusManager.ts")
    }) {
        let selected = tree.program_lookup(&file.path).unwrap();
        for (slot, symbol) in file.symbols.iter().enumerate().filter(|(_, s)| {
            matches!(
                s.name.as_str(),
                "listeners" | "Subscribable" | "FocusManager"
            )
        }) {
            let row = ids.row_id(&file.path, slot).unwrap();
            let field = selected
                .field_type_id_of(row)
                .map(|ty| format!("{:?}", arena.get(ty)));
            let input = &tree.modules.inputs[&file.path];
            println!(
                "{}",
                serde_json::json!({"source":file.path,"name":symbol.name,"row":row,"field":field,
                "base_recipes":format!("{:?}",input.bases),"info":format!("{:?}",selected.canonical_type_info(row))})
            );
        }
    }
    manifest.verify_inputs().unwrap();
}

impl Compilation {
    pub(in crate::indexer::resolve::engine) fn _test_program_modules(
        &self,
    ) -> &crate::indexer::resolve::engine::module_graph::ModuleGraph {
        &self.modules
    }
}
