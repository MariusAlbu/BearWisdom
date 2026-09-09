use super::*;

#[test]
fn constructor_heritage_rebinds_provider_edits_deletion_and_consumer_arguments() {
    let arena = Arc::new(TypeArena::new());
    let db = crate::Database::open_in_memory().unwrap();
    let provider = "interface Item { touch(): number } interface Other { other(): string } interface Base { new(base: number): Item } interface Factory extends Base { new(own: string): Other }";
    let consumer = "export {}; interface Child extends Factory {} declare const Build: Child; const actual = new Build(1); actual.touch();";
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
    let check = |tree: &Compilation,
                 file: &crate::types::ParsedFile,
                 ids: &SymbolIds,
                 target: Option<(i64, &str)>| {
        let source = file.content.as_ref().unwrap();
        let selector = source.rfind("actual.").unwrap() as u32 + 7;
        let got = call(tree, file, ids, selector);
        assert_eq!(
            got.map(|(row, ty)| (row, tree.type_arena().unwrap().format_type(ty))),
            target.map(|(row, kind)| (row, kind.to_owned()))
        );
        let lookup = tree.source_program_lookup(file).unwrap();
        let start = source.find("actual =").unwrap() as u32;
        let evidence = lookup
            .source
            .and_then(|s| {
                s.constructor_calls
                    .iter()
                    .find(|(id, _)| id.0.start == start)
            })
            .and_then(|(_, call)| call.as_ref());
        assert_eq!(evidence.is_some(), target.is_some());
        if let (Some(evidence), Some((row, _))) = (evidence, target) {
            let origin = evidence.origins[evidence.selected.unwrap()]
                .as_ref()
                .unwrap();
            assert_eq!(
                Some(origin.source),
                tree.program_lookup(lookup.symbol_by_id(row).unwrap().file_path.as_ref())
                    .unwrap()
                    .source
                    .unwrap()
                    .identity
            );
        }
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&policy(&[&files[0], &files[2]], true)),
        &HashSet::new(),
    );
    check(
        &tree,
        &files[2],
        &ids,
        Some((owner(&ids, &files[0], "touch"), "number")),
    );
    tree.persist_type_info(db.conn()).unwrap();
    let mut switched = Compilation::build_with_context(
        &[],
        &Default::default(),
        Arc::clone(&arena),
        Some(&policy(&[&files[1], &files[2]], true)),
        &HashSet::new(),
    );
    switched.ingest_from_db(db.conn());
    check(
        &switched,
        &files[2],
        &ids,
        Some((owner(&ids, &files[1], "touch"), "number")),
    );
    switched.persist_type_info(db.conn()).unwrap();
    let edit = parse(
        &arena,
        &[(
            "right.d.ts",
            &provider.replace("base: number", "base: boolean"),
        )],
    );
    let (_, edit_ids) =
        crate::indexer::write::write_parsed_files_with_origin(&db, &edit, "internal", Some(&arena))
            .unwrap();
    let mut changed = Compilation::build_with_context(
        &edit,
        &edit_ids,
        Arc::clone(&arena),
        Some(&policy(&[&edit[0], &files[2]], true)),
        &HashSet::new(),
    );
    changed.ingest_from_db(db.conn());
    check(&changed, &files[2], &ids, None);
    assert!(
        changed
            .program_lookup("main.ts")
            .unwrap()
            .symbol_by_id(owner(&edit_ids, &edit[0], "Factory"))
            .is_some(),
        "disjoint constructor overloads remain legal"
    );
    changed.persist_type_info(db.conn()).unwrap();
    let modified = parse(
        &arena,
        &[(
            "main.ts",
            &consumer
                .replace("Build(1)", "Build('yes')")
                .replace("actual.touch()", "actual.other()"),
        )],
    );
    let (_, modified_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &modified,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut consumer_edit = Compilation::build_with_context(
        &modified,
        &modified_ids,
        Arc::clone(&arena),
        Some(&policy(&[&edit[0], &modified[0]], true)),
        &HashSet::new(),
    );
    consumer_edit.ingest_from_db(db.conn());
    check(
        &consumer_edit,
        &modified[0],
        &modified_ids,
        Some((owner(&edit_ids, &edit[0], "other"), "string")),
    );
    check(&consumer_edit, &files[2], &ids, None);
    consumer_edit.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &Default::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(
        &cold,
        &modified[0],
        &modified_ids,
        Some((owner(&edit_ids, &edit[0], "other"), "string")),
    );
    db.conn()
        .execute("DELETE FROM files WHERE path='right.d.ts'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &Default::default(), Arc::clone(&restored));
    deleted.ingest_from_db(db.conn());
    check(&deleted, &modified[0], &modified_ids, None);
    let mut recovered = Compilation::build_with_context(
        &[],
        &Default::default(),
        Arc::clone(&restored),
        Some(&policy(&[&files[0], &modified[0]], true)),
        &HashSet::new(),
    );
    recovered.ingest_from_db(db.conn());
    check(
        &recovered,
        &modified[0],
        &modified_ids,
        Some((owner(&ids, &files[0], "other"), "string")),
    );
    recovered.persist_type_info(db.conn()).unwrap();
    let invalid = parse(
        &restored,
        &[(
            "left.d.ts",
            &provider.replace("base: number", "base: Missing"),
        )],
    );
    let (_, invalid_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &invalid,
        "internal",
        Some(&restored),
    )
    .unwrap();
    let mut rejected = Compilation::build_with_context(
        &invalid,
        &invalid_ids,
        Arc::clone(&restored),
        Some(&policy(&[&invalid[0], &modified[0]], true)),
        &HashSet::new(),
    );
    rejected.ingest_from_db(db.conn());
    check(&rejected, &modified[0], &modified_ids, None);
    assert!(
        rejected
            .program_lookup("main.ts")
            .unwrap()
            .symbol_by_id(owner(&modified_ids, &modified[0], "Child"))
            .is_none(),
        "unknown base invalidates dependent heritage"
    );
    rejected.persist_type_info(db.conn()).unwrap();
    let next = Arc::new(TypeArena::new());
    next.restore_snapshot(&restored.serialize_snapshot());
    let mut final_cold = Compilation::build(&[], &Default::default(), next);
    final_cold.ingest_from_db(db.conn());
    check(&final_cold, &modified[0], &modified_ids, None);
}

#[test]
fn constructor_variance_tracks_policy_and_does_not_use_method_bivariance() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("main.ts", "interface Item {} interface Source { new(x: number): Item } interface Target { new(x: number | string): Item }")]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let a = owner(&ids, &files[0], "Source");
    let b = owner(&ids, &files[0], "Target");
    let mut prior = None;
    for (strict, methods, present) in [
        (true, Some(true), true),
        (false, Some(true), true),
        (true, Some(false), true),
        (true, None, true),
        (true, Some(true), false),
    ] {
        let mut config = policy(&[&files[0]], strict);
        if present {
            config.programs.as_mut().unwrap()[0]
                .callable_policy
                .as_mut()
                .unwrap()
                .bivariant_methods = methods;
        } else {
            config.programs.as_mut().unwrap()[0].callable_policy = None;
        }
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
        let check = |tree: &Compilation| {
            let lookup = tree.program_lookup("main.ts").unwrap();
            let arena = tree.type_arena().unwrap();
            let left = (&lookup as &dyn SymbolLookup)
                .declaration_type(arena, a)
                .unwrap();
            let right = (&lookup as &dyn SymbolLookup)
                .declaration_type(arena, b)
                .unwrap();
            assert_eq!(
                types::Relation {
                    lookup: &lookup,
                    arena
                }
                .argument(left, right),
                present.then_some(!strict)
            );
            left
        };
        if let Some(old) = prior {
            assert_eq!(
                tree.program_lookup("main.ts")
                    .unwrap()
                    .evaluated_receiver(old),
                Some(None)
            );
        }
        prior = Some(check(&tree));
        tree.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &Default::default(), restored);
        cold.ingest_from_db(db.conn());
        check(&cold);
    }
}
