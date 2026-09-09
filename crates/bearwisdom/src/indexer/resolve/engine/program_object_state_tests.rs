use super::*;
use crate::indexer::symbol_ids::SymbolIds;

fn resolved(
    tree: &Compilation,
    file: &crate::types::ParsedFile,
    ids: &SymbolIds,
) -> Option<(i64, String)> {
    let lookup = FileLookup::for_file(tree, file, ids);
    let reference = file
        .refs
        .iter()
        .find(|r| {
            r.kind == crate::types::EdgeKind::Calls
                && r.chain
                    .as_ref()
                    .is_some_and(|c| c.segments.last().unwrap().name == "read")
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
        SolveOutcome::Resolved(info) => Some((
            info.target_symbol_id,
            tree.type_arena()
                .unwrap()
                .format_type(info.resolved_yield_type?),
        )),
        _ => None,
    }
}

#[test]
fn object_origins_follow_config_provider_edits_deletion_and_stale_source() {
    let arena = Arc::new(TypeArena::new());
    let db = crate::Database::open_in_memory().unwrap();
    let provider = "const api = (() => { return { read(): number { return 1 } }; })();";
    let main = "export {}; api.read();";
    let files = parse(
        &arena,
        &[
            ("left.ts", provider),
            ("right.ts", provider),
            ("main.ts", main),
        ],
    );
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let config = |p: &crate::types::ParsedFile, main: &crate::types::ParsedFile| {
        let mut c = context(&[p, main]);
        c.programs.as_mut().unwrap()[0].callable_policy = Some(CallablePolicy {
            strict_parameters: true,
            strict_nulls: true,
            bivariant_methods: Some(true),
        });
        c
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config(&files[0], &files[2])),
        &HashSet::new(),
    );
    assert_eq!(
        resolved(&tree, &files[2], &ids),
        Some((owner(&ids, &files[0], "read"), "number".into()))
    );
    let old = tree
        .program_lookup("main.ts")
        .unwrap()
        .field_type_id_of(owner(&ids, &files[0], "api"))
        .unwrap();
    tree.persist_type_info(db.conn()).unwrap();
    let mut switched = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&arena),
        Some(&config(&files[1], &files[2])),
        &HashSet::new(),
    );
    switched.ingest_from_db(db.conn());
    assert_eq!(
        resolved(&switched, &files[2], &ids),
        Some((owner(&ids, &files[1], "read"), "number".into()))
    );
    assert!(
        !(&switched.program_lookup("main.ts").unwrap() as &dyn SymbolLookup)
            .accepts_type_context(&arena, old)
    );
    switched.persist_type_info(db.conn()).unwrap();
    let edited = parse(
        &arena,
        &[(
            "right.ts",
            &provider.replace("number { return 1", "string { return 'new'"),
        )],
    );
    let (_, edit_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &edited,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut changed = Compilation::build_with_context(
        &edited,
        &edit_ids,
        Arc::clone(&arena),
        Some(&config(&edited[0], &files[2])),
        &HashSet::new(),
    );
    changed.ingest_from_db(db.conn());
    assert_eq!(
        resolved(&changed, &files[2], &ids),
        Some((owner(&edit_ids, &edited[0], "read"), "string".into()))
    );
    changed.persist_type_info(db.conn()).unwrap();
    let cold_arena = Arc::new(TypeArena::new());
    cold_arena.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&cold_arena));
    cold.ingest_from_db(db.conn());
    assert_eq!(
        resolved(&cold, &files[2], &ids),
        Some((owner(&edit_ids, &edited[0], "read"), "string".into()))
    );
    db.conn()
        .execute("DELETE FROM files WHERE path='right.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&cold_arena));
    deleted.ingest_from_db(db.conn());
    assert_eq!(resolved(&deleted, &files[2], &ids), None);
    let mut recovered = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&cold_arena),
        Some(&config(&files[0], &files[2])),
        &HashSet::new(),
    );
    recovered.ingest_from_db(db.conn());
    assert_eq!(
        resolved(&recovered, &files[2], &ids),
        Some((owner(&ids, &files[0], "read"), "number".into()))
    );
    recovered.persist_type_info(db.conn()).unwrap();
    let consumer = parse(&cold_arena, &[("main.ts", "export {}; api.read('bad');")]);
    let (_, consumer_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &consumer,
        "internal",
        Some(&cold_arena),
    )
    .unwrap();
    let mut invalid = Compilation::build_with_context(
        &consumer,
        &consumer_ids,
        Arc::clone(&cold_arena),
        Some(&config(&files[0], &consumer[0])),
        &HashSet::new(),
    );
    invalid.ingest_from_db(db.conn());
    assert_eq!(resolved(&invalid, &consumer[0], &consumer_ids), None);
    assert_eq!(resolved(&invalid, &files[2], &ids), None);
    invalid.persist_type_info(db.conn()).unwrap();
    let final_arena = Arc::new(TypeArena::new());
    final_arena.restore_snapshot(&cold_arena.serialize_snapshot());
    let mut final_cold = Compilation::build(&[], &SymbolIds::default(), final_arena);
    final_cold.ingest_from_db(db.conn());
    assert_eq!(resolved(&final_cold, &consumer[0], &consumer_ids), None);
    assert_eq!(resolved(&final_cold, &files[2], &ids), None);
}

#[test]
fn rowless_object_members_keep_type_and_signature_without_fabricating_navigation() {
    let arena = Arc::new(TypeArena::new());
    let source = "export const api = { read(): number { return 1 } }; api.read();";
    let mut files = parse(&arena, &[("main.ts", source)]);
    for object in &mut files[0].flow.lexical.as_mut().unwrap().types.objects {
        for member in object.members.as_mut().unwrap() {
            member.declaration = None;
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
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0]])),
        &HashSet::new(),
    );
    let lookup = FileLookup::for_file(&tree, &files[0], &ids);
    lookup.set_cursor(source.rfind("api.read").unwrap() as u32);
    let receiver = lookup.local_type_id("api").unwrap();
    let selector = source.rfind("read()").unwrap() as u32;
    let call = lookup
        .overloaded_call(receiver, selector, &[], &[])
        .unwrap()
        .ok()
        .unwrap();
    assert_eq!(call.origins[0].declaration, None);
    assert_eq!(
        call.origins[0].span.start,
        source.find("read():").unwrap() as u32
    );
    assert_eq!(arena.format_type(call.return_type), "number");
    assert_eq!(resolved(&tree, &files[0], &ids), None);
}
