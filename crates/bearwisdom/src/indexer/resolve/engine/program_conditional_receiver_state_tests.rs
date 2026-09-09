use super::*;

fn selected(
    tree: &Compilation,
    file: &crate::types::ParsedFile,
    ids: &SymbolIds,
) -> Option<(i64, String)> {
    let source = file.content.as_ref().unwrap();
    let selector = source.rfind("touch()").unwrap() as u32;
    let SolveOutcome::Resolved(info) = outcome(tree, file, ids, selector) else {
        return None;
    };
    let arena = tree.type_arena().unwrap();
    Some((
        info.target_symbol_id,
        arena.format_type(info.resolved_yield_type?),
    ))
}

#[test]
fn conditional_receivers_rebuild_for_configuration_provider_and_consumer_changes() {
    let arena = Arc::new(TypeArena::new());
    let db = crate::Database::open_in_memory().unwrap();
    let provider = "interface Yes { touch(): number } interface No { touch(): string } type Choose<T extends string> = T extends 'yes' ? Yes : No;";
    let consumer = "export {}; declare const x: Choose<'yes'>; x.touch();";
    let files = parse(
        &arena,
        &[
            ("left.d.ts", provider),
            ("right.d.ts", provider),
            ("main.ts", consumer),
        ],
    );
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let target = |ids: &SymbolIds, file: &crate::types::ParsedFile, kind: &str| {
        let start = file
            .content
            .as_ref()
            .unwrap()
            .find(&format!("touch(): {kind}"))
            .unwrap();
        let slot = file
            .symbols
            .iter()
            .position(|s| s.start_line == 0 && s.start_col as usize == start)
            .unwrap();
        ids.row_id(&file.path, slot).unwrap()
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&policy(&[&files[0], &files[2]])),
        &HashSet::new(),
    );
    assert_eq!(
        selected(&tree, &files[2], &ids),
        Some((target(&ids, &files[0], "number"), "number".into()))
    );
    let old_lookup = tree.program_lookup("main.ts").unwrap();
    let old = old_lookup
        .field_type_id_of(owner(&ids, &files[2], "x"))
        .unwrap();
    tree.persist_type_info(db.conn()).unwrap();
    let mut switched = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&arena),
        Some(&policy(&[&files[1], &files[2]])),
        &HashSet::new(),
    );
    switched.ingest_from_db(db.conn());
    assert_eq!(
        selected(&switched, &files[2], &ids),
        Some((target(&ids, &files[1], "number"), "number".into()))
    );
    assert_eq!(
        switched
            .program_lookup("main.ts")
            .unwrap()
            .evaluated_receiver(old),
        Some(None)
    );
    switched.persist_type_info(db.conn()).unwrap();
    let edited = parse(
        &arena,
        &[("right.d.ts", &provider.replace("? Yes : No", "? No : Yes"))],
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
        Some(&policy(&[&edited[0], &files[2]])),
        &HashSet::new(),
    );
    changed.ingest_from_db(db.conn());
    assert_eq!(
        selected(&changed, &files[2], &ids),
        Some((target(&edit_ids, &edited[0], "string"), "string".into()))
    );
    changed.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    assert_eq!(
        selected(&cold, &files[2], &ids),
        Some((target(&edit_ids, &edited[0], "string"), "string".into()))
    );
    db.conn()
        .execute("DELETE FROM files WHERE path='right.d.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    deleted.ingest_from_db(db.conn());
    assert_eq!(selected(&deleted, &files[2], &ids), None);
    let mut recovered = Compilation::build_with_context(
        &[],
        &SymbolIds::default(),
        Arc::clone(&restored),
        Some(&policy(&[&files[0], &files[2]])),
        &HashSet::new(),
    );
    recovered.ingest_from_db(db.conn());
    assert_eq!(
        selected(&recovered, &files[2], &ids),
        Some((target(&ids, &files[0], "number"), "number".into()))
    );
    recovered.persist_type_info(db.conn()).unwrap();
    let consumer = parse(
        &restored,
        &[(
            "main.ts",
            "export {}; declare const x: Choose<number>; x.touch();",
        )],
    );
    let (_, consumer_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &consumer,
        "internal",
        Some(&restored),
    )
    .unwrap();
    let mut invalid = Compilation::build_with_context(
        &consumer,
        &consumer_ids,
        Arc::clone(&restored),
        Some(&policy(&[&files[0], &consumer[0]])),
        &HashSet::new(),
    );
    invalid.ingest_from_db(db.conn());
    assert_eq!(selected(&invalid, &consumer[0], &consumer_ids), None);
    assert_eq!(selected(&invalid, &files[2], &ids), None);
    invalid.persist_type_info(db.conn()).unwrap();
    let final_arena = Arc::new(TypeArena::new());
    final_arena.restore_snapshot(&restored.serialize_snapshot());
    let mut final_cold = Compilation::build(&[], &SymbolIds::default(), final_arena);
    final_cold.ingest_from_db(db.conn());
    assert_eq!(selected(&final_cold, &consumer[0], &consumer_ids), None);
}
