use super::*;

fn parse(arena: &TypeArena, sources: &[(&str, &str)]) -> Vec<ParsedFile> {
    let dir = tempfile::tempdir().unwrap();
    sources
        .iter()
        .map(|(path, source)| {
            let absolute = dir.path().join(path);
            std::fs::write(&absolute, source).unwrap();
            crate::indexer::parse_file::parse_file_with_arena(
                &crate::walker::WalkedFile {
                    relative_path: (*path).into(),
                    absolute_path: absolute,
                    language: "rust",
                },
                &crate::languages::default_registry(),
                arena,
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn owner_recipe_retargeting_and_deletion_rebuild_member_and_self_return_ids() {
    let arena = Arc::new(TypeArena::new());
    let db = crate::Database::open_in_memory().unwrap();
    let parsed = parse(&arena, &[("lib.rs", "mod a { pub struct Item; } mod b { pub struct Item; }
        mod provider { pub use super::a::Item as Target; }
        mod body { impl Alias { pub fn make() -> Self { loop {} } } use super::provider::Target as Alias; }")]);
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut tree = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    tree.ingest(&parsed, &ids, &Default::default());
    let source = &parsed[0];
    let row = |name: &str| {
        ids.row_id(
            &source.path,
            source
                .symbols
                .iter()
                .position(|s| s.qualified_name == name)
                .unwrap(),
        )
        .unwrap()
    };
    let (a, b, method) = (row("a.Item"), row("b.Item"), row("body.Alias.make"));
    assert_eq!(
        tree.members_of_id(a)
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        [method]
    );
    assert_eq!(
        super::super::super::head_decl::head_decl_id(
            &arena,
            tree.return_type_id_of(method).unwrap()
        ),
        Some(a)
    );
    // Retarget the provider's ingestion path, not a declaration or member name.
    let input = tree.modules.inputs.get_mut("lib.rs").unwrap();
    for import in &mut input.imports {
        if let super::super::super::module_input::InputTarget::ContextPath { selectors, .. } =
            &mut import.target
        {
            if let Some((name, _)) = selectors.first_mut().filter(|(name, _)| name == "a") {
                *name = "b".into();
            }
        }
    }
    let mut modules = std::mem::take(&mut tree.modules);
    modules.rebuild(&tree);
    tree.modules = modules;
    tree.rebind_extension_owners();
    assert!(tree.members_of_id(a).is_empty());
    assert_eq!(
        tree.members_of_id(b)
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        [method]
    );
    tree.persist_type_info(db.conn()).unwrap();
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    cold.ingest_from_db(db.conn());
    assert_eq!(
        cold.members_of_id(b)
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        [method]
    );
    assert_eq!(
        super::super::super::head_decl::head_decl_id(
            &arena,
            cold.return_type_id_of(method).unwrap()
        ),
        Some(b)
    );
    cold.modules.inputs.clear();
    cold.rebind_extension_owners();
    assert!(cold.members_of_id(b).is_empty());
}

#[test]
fn unknown_and_changed_extension_owners_remove_previous_membership() {
    let mut tree = Compilation::build(&[], &SymbolIds::default(), Arc::new(TypeArena::new()));
    tree.extension_owners.insert(71, Some(12));
    tree.members_by_id.insert(12, vec![71, 72]);
    tree.enclosing_type_by_id.insert(71, 12);
    tree.rebind_extension_owners();
    assert_eq!(tree.members_by_id[&12], vec![72]);
    assert!(!tree.enclosing_type_by_id.contains_key(&71));
    assert!(tree.extension_owners.is_empty());
}

#[test]
fn actual_provider_edit_and_deletion_rebind_unchanged_cross_file_impls() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='sample'\n[lib]\npath='lib.rs'",
    )
    .unwrap();
    std::fs::write(dir.path().join("lib.rs"), "").unwrap();
    let context = crate::indexer::project_context::build_project_context(dir.path());
    let arena = Arc::new(TypeArena::new());
    let db = crate::Database::open_in_memory().unwrap();
    let parsed = parse(&arena, &[("lib.rs", "mod a; mod b; mod provider; mod body;"),
        ("a.rs", "pub struct Item;"), ("b.rs", "pub struct Item;"),
        ("provider.rs", "pub use crate::a::Item as Target;"),
        ("body.rs", "use crate::provider::Target as Alias; impl Alias { pub fn make() -> Self { loop {} } }")]);
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut tree = Compilation::build_with_context(
        &parsed[..4],
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    tree.ingest(&parsed[4..], &ids, &Default::default());
    let row = |file: usize, name: &str| {
        ids.row_id(
            &parsed[file].path,
            parsed[file]
                .symbols
                .iter()
                .position(|s| s.name == name)
                .unwrap(),
        )
        .unwrap()
    };
    let (a, b, method) = (row(1, "Item"), row(2, "Item"), row(4, "make"));
    assert_eq!(
        tree.members_of_id(a)
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        [method]
    );
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(
        &arena,
        &[("provider.rs", "pub use crate::b::Item as Target;")],
    );
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut incremental = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    incremental.ingest_from_db(db.conn());
    assert!(incremental.members_of_id(a).is_empty());
    assert_eq!(
        incremental
            .members_of_id(b)
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        [method]
    );
    assert_eq!(
        super::super::super::head_decl::head_decl_id(
            &arena,
            incremental.return_type_id_of(method).unwrap()
        ),
        Some(b)
    );
    incremental.persist_type_info(db.conn()).unwrap();
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    cold.ingest_from_db(db.conn());
    assert_eq!(
        cold.members_of_id(b)
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        [method]
    );
    db.conn()
        .execute("DELETE FROM files WHERE path='b.rs'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    deleted.ingest_from_db(db.conn());
    assert!(deleted.members_of_id(a).is_empty());
    assert!(deleted.members_of_id(b).is_empty());
    assert_eq!(
        arena.get(deleted.return_type_id_of(method).unwrap()),
        Type::Unknown
    );
}

#[test]
fn alias_provider_edits_rebind_receiver_constraints_without_editing_impl_or_caller() {
    use crate::type_checker::core::types::PrimKind;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='sample'\n[lib]\npath='lib.rs'",
    )
    .unwrap();
    std::fs::write(dir.path().join("lib.rs"), "").unwrap();
    let context = crate::indexer::project_context::build_project_context(dir.path());
    let arena = Arc::new(TypeArena::new());
    let db = crate::Database::open_in_memory().unwrap();
    let parsed = parse(&arena, &[("lib.rs", "mod model; mod provider; mod body;"),
        ("model.rs", "pub struct Item<T> { pub inner: T }"),
        ("provider.rs", "pub type Target = crate::model::Item<i32>;"),
        ("body.rs", "use crate::provider::Target as Alias; impl Alias { pub fn make() -> Self { loop {} } }")]);
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut tree = Compilation::build_with_context(
        &parsed[..3],
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    tree.ingest(&parsed[3..], &ids, &Default::default());
    let row = |file: usize, name: &str| {
        ids.row_id(
            &parsed[file].path,
            parsed[file]
                .symbols
                .iter()
                .position(|s| s.name == name)
                .unwrap(),
        )
        .unwrap()
    };
    let (owner, method) = (row(1, "Item"), row(3, "make"));
    let base = arena.decl("model.Item", owner);
    let app = |p| {
        arena.intern(Type::Apply {
            base,
            args: vec![arena.primitive(p)],
        })
    };
    let verify = |tree: &Compilation, accepted, rejected| {
        let pattern = tree
            .member_pattern(method)
            .expect("receiver restriction survives reload");
        assert!(matches!(
            pattern.bindings(tree, &arena, app(accepted)),
            Ok(Some(_))
        ));
        assert_eq!(pattern.bindings(tree, &arena, app(rejected)), Ok(None));
        assert_eq!(
            super::super::super::contract::member_applicability::expand(
                tree,
                &arena,
                tree.return_type_id_of(method).unwrap()
            ),
            Some(app(accepted))
        );
        let name = tree.member_index.name("make").unwrap();
        assert_eq!(
            super::super::super::member_selection::select(tree, owner, name, &|_| true),
            super::super::super::member_selection::Selection::Incomplete
        );
    };
    verify(&tree, PrimKind::Signed(32), PrimKind::Unsigned(32));
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(
        &arena,
        &[("provider.rs", "pub type Target = crate::model::Item<u32>;")],
    );
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut incremental = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    incremental.ingest_from_db(db.conn());
    verify(&incremental, PrimKind::Unsigned(32), PrimKind::Signed(32));
    incremental.persist_type_info(db.conn()).unwrap();
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    cold.ingest_from_db(db.conn());
    verify(&cold, PrimKind::Unsigned(32), PrimKind::Signed(32));
    db.conn()
        .execute("DELETE FROM files WHERE path='model.rs'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    deleted.ingest_from_db(db.conn());
    assert!(deleted.member_pattern(method).is_none());
    assert!(!deleted.members_of_id(owner).iter().any(|s| s.id == method));
}

#[test]
fn referent_provider_edit_retargets_unchanged_alias_and_impl_and_deletion_cannot_reuse_old_ids() {
    use crate::type_checker::core::types::{Indirection, Lifetime, Mutability};
    for (alias, body) in [
        ("pub type Target = crate::model::Item<&'static crate::provider::Doc>;",
            "use crate::aliases::Target as Alias; impl Alias { pub fn make() -> Self { loop {} } }"),
        ("pub type Target<'a> = crate::model::Item<&'a crate::provider::Doc>;",
            "use crate::aliases::Target as Alias; impl<'x> Alias<'x> { pub fn make(&self) -> Self { loop {} } }"),
    ] {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='sample'\n[lib]\npath='lib.rs'").unwrap();
    std::fs::write(dir.path().join("lib.rs"), "").unwrap();
    let context = crate::indexer::project_context::build_project_context(dir.path());
    let arena = Arc::new(TypeArena::new()); let db = crate::Database::open_in_memory().unwrap();
    let parsed = parse(&arena, &[("lib.rs", "mod model; mod a; mod b; mod provider; mod aliases; mod body;"),
        ("model.rs", "pub struct Item<T> { pub inner: T }"), ("a.rs", "pub struct Doc;"), ("b.rs", "pub struct Doc;"),
        ("provider.rs", "pub use crate::a::Doc;"),
        ("aliases.rs", alias), ("body.rs", body)]);
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(&db, &parsed, "internal", Some(&arena)).unwrap();
    let mut tree = Compilation::build_with_context(&parsed[..6], &ids, Arc::clone(&arena), Some(&context), &Default::default());
    tree.ingest(&parsed[6..], &ids, &Default::default());
    let row = |file: usize, name: &str| ids.row_id(&parsed[file].path, parsed[file].symbols.iter().position(|s| s.name == name).unwrap()).unwrap();
    let (owner, a, b, method) = (row(1, "Item"), row(2, "Doc"), row(3, "Doc"), row(6, "make"));
    let base = arena.decl("model.Item", owner);
    let app = |id| arena.intern(Type::Apply { base, args: vec![arena.intern(Type::Indirect {
        kind: Indirection::Reference(Lifetime::Static), mutability: Mutability::Shared, inner: arena.decl("Doc", id) })] });
    let verify = |tree: &Compilation, accepted, rejected| {
        let pattern = tree.member_pattern(method).expect("receiver restriction");
        assert!(matches!(pattern.bindings(tree, &arena, app(accepted)), Ok(Some(_))));
        assert_eq!(pattern.bindings(tree, &arena, app(rejected)), Ok(None));
        let yielded = super::super::super::bound_call::member_yield(tree, &arena, method, app(accepted), tree.return_type_id_of(method).unwrap());
        assert_eq!(super::super::super::contract::member_applicability::expand(tree, &arena, yielded),
            Some(app(accepted)));
    };
    verify(&tree, a, b); tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(&arena, &[("provider.rs", "pub use crate::b::Doc;")]);
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(&db, &changed, "internal", Some(&arena)).unwrap();
    let mut incremental = Compilation::build_with_context(&changed, &changed_ids, Arc::clone(&arena), Some(&context), &Default::default());
    incremental.ingest_from_db(db.conn()); verify(&incremental, b, a); incremental.persist_type_info(db.conn()).unwrap();
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena)); cold.ingest_from_db(db.conn());
    verify(&cold, b, a);
    db.conn().execute("DELETE FROM files WHERE path='b.rs'", []).unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena)); deleted.ingest_from_db(db.conn());
    for id in [a, b] {
        assert!(!matches!(deleted.member_pattern(method).map(|pattern| pattern.bindings(&deleted, &arena, app(id))), Some(Ok(Some(_)))));
    }
    }
}

#[test]
fn changing_a_provider_parameter_kind_invalidates_unchanged_impl_members_fresh_and_cold() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='sample'\n[lib]\npath='lib.rs'",
    )
    .unwrap();
    std::fs::write(dir.path().join("lib.rs"), "").unwrap();
    let context = crate::indexer::project_context::build_project_context(dir.path());
    let arena = Arc::new(TypeArena::new());
    let db = crate::Database::open_in_memory().unwrap();
    let parsed = parse(&arena, &[("lib.rs", "mod model; mod provider; mod body;"),
        ("model.rs", "pub struct Item<T> { pub inner: T } pub struct Doc;"),
        ("provider.rs", "pub type Target<'a> = crate::model::Item<&'a crate::model::Doc>;"),
        ("body.rs", "use crate::provider::Target as Alias; impl<'x> Alias<'x> { pub fn make(&self) -> Self { loop {} } }")]);
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut tree = Compilation::build_with_context(
        &parsed,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    let row = |file: usize, name: &str| {
        ids.row_id(
            &parsed[file].path,
            parsed[file]
                .symbols
                .iter()
                .position(|s| s.name == name)
                .unwrap(),
        )
        .unwrap()
    };
    let (owner, method) = (row(1, "Item"), row(3, "make"));
    assert!(tree.member_pattern(method).is_some());
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(
        &arena,
        &[("provider.rs", "pub type Target<T> = crate::model::Item<T>;")],
    );
    let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &changed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut incremental = Compilation::build_with_context(
        &changed,
        &changed_ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    incremental.ingest_from_db(db.conn());
    assert!(incremental.member_pattern(method).is_none());
    assert!(!incremental
        .members_of_id(owner)
        .iter()
        .any(|m| m.id == method));
    incremental.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    let snapshot: String = db
        .conn()
        .query_row(
            "SELECT value FROM _bearwisdom_meta WHERE key='type_arena_snapshot'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    restored.restore_snapshot(&snapshot);
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    assert!(cold.member_pattern(method).is_none());
    assert!(!cold.members_of_id(owner).iter().any(|m| m.id == method));
}

#[test]
fn elided_input_slots_rebind_on_late_provider_arity_kind_edit_and_deletion() {
    use crate::type_checker::core::types::Lifetime;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='sample'\n[lib]\npath='lib.rs'",
    )
    .unwrap();
    std::fs::write(dir.path().join("lib.rs"), "").unwrap();
    let context = crate::indexer::project_context::build_project_context(dir.path());
    let arena = Arc::new(TypeArena::new());
    let db = crate::Database::open_in_memory().unwrap();
    let provider = "pub type Target<'a,T> = (&'a crate::model::Doc,T);";
    let parsed = parse(
        &arena,
        &[
            ("lib.rs", "mod model; mod provider; mod body;"),
            ("model.rs", "pub struct Doc;"),
            ("provider.rs", provider),
            (
                "body.rs",
                "use crate::provider::Target; pub fn f(p: Target<crate::model::Doc>) {}",
            ),
        ],
    );
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let row = |file: usize, name: &str| {
        ids.row_id(
            &parsed[file].path,
            parsed[file]
                .symbols
                .iter()
                .position(|s| s.name == name)
                .unwrap(),
        )
        .unwrap()
    };
    let (doc, function, local) = (row(1, "Doc"), row(3, "f"), row(3, "p"));
    let mut tree = Compilation::build_with_context(
        &parsed[..2],
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    tree.ingest(&parsed[3..], &ids, &Default::default());
    let verify = |tree: &Compilation, count: usize, target: i64| {
        let info = tree.canonical_type_info(function).unwrap();
        let ty = info.parameter_type_ids.as_ref().unwrap()[0];
        assert_eq!(
            tree.field_type_id_of(local),
            Some(ty),
            "annotation and signature share the source site"
        );
        let Type::Apply { base, args } = tree.arena.get(ty) else {
            panic!("input lost its application");
        };
        assert_eq!(
            super::super::super::head_decl::head_decl_id(&tree.arena, base),
            Some(target)
        );
        assert_eq!(args.len(), count + 1);
        assert_eq!(info.elided_input_params.len(), count);
        assert!(
            info.generic_param_ids.is_empty(),
            "anonymous slots do not become explicit type arguments"
        );
        let regions: Vec<_> = args[..count]
            .iter()
            .map(|&arg| {
                let Type::Region(Lifetime::Parameter(p)) = tree.arena.get(arg) else {
                    panic!("omitted region must be an ID");
                };
                p
            })
            .collect();
        for (index, &p) in regions.iter().enumerate() {
            assert_eq!(info.elided_input_params[index].1, index);
            assert_eq!(info.elided_input_params[index].2, p);
            assert!(!regions[..index].contains(&p), "omitted slots are distinct");
        }
        assert_eq!(
            super::super::super::head_decl::head_decl_id(&tree.arena, args[count]),
            Some(doc)
        );
        regions
    };
    assert!(
        tree.canonical_type_info(function)
            .unwrap()
            .elided_input_params
            .is_empty(),
        "no guessed provider arity"
    );
    tree.ingest(&parsed[2..3], &ids, &Default::default());
    let initial = verify(&tree, 1, row(2, "Target"));
    tree.persist_type_info(db.conn()).unwrap();
    for (source, count) in [
        (
            "pub type Target<'a,'b,T> = (&'a crate::model::Doc,&'b crate::model::Doc,T);",
            2,
        ),
        ("pub type Target<T> = (T,);", 0),
        (provider, 1),
    ] {
        let changed = parse(&arena, &[("provider.rs", source)]);
        let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &changed,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let mut incremental = Compilation::build_with_context(
            &changed,
            &changed_ids,
            Arc::clone(&arena),
            Some(&context),
            &Default::default(),
        );
        incremental.ingest_from_db(db.conn());
        let target = changed_ids
            .row_id(
                &changed[0].path,
                changed[0]
                    .symbols
                    .iter()
                    .position(|s| s.name == "Target")
                    .unwrap(),
            )
            .unwrap();
        let active = verify(&incremental, count, target);
        if count == 2 {
            assert_eq!(
                active[0], initial[0],
                "an unchanged source slot retains its ID"
            );
        }
        incremental.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        let snapshot: String = db
            .conn()
            .query_row(
                "SELECT value FROM _bearwisdom_meta WHERE key='type_arena_snapshot'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        restored.restore_snapshot(&snapshot);
        let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
        cold.ingest_from_db(db.conn());
        assert_eq!(
            verify(&cold, count, target),
            active,
            "cold snapshot preserves anonymous IDs"
        );
    }
    db.conn()
        .execute("DELETE FROM files WHERE path='provider.rs'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    deleted.ingest_from_db(db.conn());
    let info = deleted.canonical_type_info(function).unwrap();
    assert!(info.elided_input_params.is_empty());
    let ty = info.parameter_type_ids.as_ref().unwrap()[0];
    assert_eq!(
        super::super::super::head_decl::head_decl_id(&arena, ty),
        None,
        "deleted owner cannot survive in an application"
    );
}

#[test]
fn output_elision_rebinds_provider_positions_without_stale_regions_or_shifted_type_arguments() {
    use crate::type_checker::core::types::Lifetime;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='sample'\n[lib]\npath='lib.rs'",
    )
    .unwrap();
    std::fs::write(dir.path().join("lib.rs"), "").unwrap();
    let context = crate::indexer::project_context::build_project_context(dir.path());
    let arena = Arc::new(TypeArena::new());
    let db = crate::Database::open_in_memory().unwrap();
    let provider =
        "pub type Input<'a> = &'a crate::model::Doc; pub struct Output<'a,T> { inner: &'a T }";
    let parsed = parse(
        &arena,
        &[
            ("lib.rs", "mod model; mod provider; mod body;"),
            ("model.rs", "pub struct Doc;"),
            ("provider.rs", provider),
            (
                "body.rs",
                "use crate::provider::{Input,Output}; use crate::model::Doc;
            pub fn make(p: Input) -> Output<Doc> { loop {} }",
            ),
        ],
    );
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let row = |file: usize, name: &str| {
        ids.row_id(
            &parsed[file].path,
            parsed[file]
                .symbols
                .iter()
                .position(|s| s.name == name)
                .unwrap(),
        )
        .unwrap()
    };
    let (doc, function) = (row(1, "Doc"), row(3, "make"));
    let mut tree = Compilation::build_with_context(
        &parsed[..2],
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    tree.ingest(&parsed[3..], &ids, &Default::default());
    let verify = |tree: &Compilation, inputs: usize, outputs: usize, target: i64| {
        let info = tree.canonical_type_info(function).unwrap();
        assert_eq!(
            info.elided_input_params.len(),
            inputs,
            "outputs do not allocate anonymous parameters"
        );
        let Type::Apply { base, args } = tree.arena.get(info.return_type_id.unwrap()) else {
            panic!("output application");
        };
        assert_eq!(
            super::super::super::head_decl::head_decl_id(&tree.arena, base),
            Some(target)
        );
        assert_eq!(args.len(), outputs + 1);
        let expected = if inputs == 1 {
            Lifetime::Parameter(info.elided_input_params[0].2)
        } else {
            Lifetime::Unknown
        };
        for &arg in &args[..outputs] {
            assert_eq!(tree.arena.get(arg), Type::Region(expected));
        }
        assert_eq!(
            super::super::super::head_decl::head_decl_id(&tree.arena, args[outputs]),
            Some(doc)
        );
        expected
    };
    assert!(tree
        .canonical_type_info(function)
        .unwrap()
        .elided_input_params
        .is_empty());
    tree.ingest(&parsed[2..3], &ids, &Default::default());
    verify(&tree, 1, 1, row(2, "Output"));
    tree.persist_type_info(db.conn()).unwrap();
    for (source, inputs, outputs) in [
        ("pub type Input<'a> = &'a crate::model::Doc; pub struct Output<'a,'b,T> { a: &'a T, b: &'b T }", 1, 2),
        ("pub type Input<'a,'b> = (&'a crate::model::Doc,&'b crate::model::Doc); pub struct Output<'a,'b,T> { a: &'a T, b: &'b T }", 2, 2),
        ("pub type Input = &'static crate::model::Doc; pub struct Output<'a,T> { a: &'a T }", 0, 1),
        ("pub type Input<'a> = crate::model::Doc; pub struct Output<'a,T> { a: &'a T }", 1, 1),
    ] {
        let changed = parse(&arena, &[("provider.rs", source)]);
        let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(&db, &changed, "internal", Some(&arena)).unwrap();
        let mut incremental = Compilation::build_with_context(&changed, &changed_ids, Arc::clone(&arena), Some(&context), &Default::default());
        incremental.ingest_from_db(db.conn());
        let target = changed_ids.row_id(&changed[0].path, changed[0].symbols.iter().position(|s| s.name == "Output").unwrap()).unwrap();
        let region = verify(&incremental, inputs, outputs, target); incremental.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        let snapshot: String = db.conn().query_row("SELECT value FROM _bearwisdom_meta WHERE key='type_arena_snapshot'", [], |r| r.get(0)).unwrap();
        restored.restore_snapshot(&snapshot);
        let mut cold = Compilation::build(&[], &SymbolIds::default(), restored); cold.ingest_from_db(db.conn());
        assert_eq!(verify(&cold, inputs, outputs, target), region);
    }
    db.conn()
        .execute("DELETE FROM files WHERE path='provider.rs'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    deleted.ingest_from_db(db.conn());
    let info = deleted.canonical_type_info(function).unwrap();
    assert!(info.elided_input_params.is_empty());
    assert_eq!(
        super::super::super::head_decl::head_decl_id(&arena, info.return_type_id.unwrap()),
        None
    );
}

#[test]
fn receiver_signature_rebinds_nominal_provider_and_region_ids_across_edits_and_cold_reload() {
    use crate::type_checker::core::types::{Indirection, Lifetime};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='sample'\n[lib]\npath='lib.rs'",
    )
    .unwrap();
    std::fs::write(dir.path().join("lib.rs"), "").unwrap();
    let context = crate::indexer::project_context::build_project_context(dir.path());
    let arena = Arc::new(TypeArena::new());
    let db = crate::Database::open_in_memory().unwrap();
    let parsed = parse(&arena, &[("lib.rs", "mod a; mod b; mod provider; mod body; mod caller;"),
        ("a.rs", "pub struct C;"), ("b.rs", "pub struct C;"), ("provider.rs", "pub use crate::a::C;"),
        ("body.rs", "use crate::provider::C; impl C { pub fn make(&self, other: &crate::b::C) -> &Self { loop {} } }"),
        ("caller.rs", "use crate::provider::C; fn f(p:C, other:&crate::b::C) { p.make(other); }")]);
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let row = |file: usize, name: &str| {
        ids.row_id(
            &parsed[file].path,
            parsed[file]
                .symbols
                .iter()
                .position(|s| s.name == name)
                .unwrap(),
        )
        .unwrap()
    };
    let (a, b, method) = (row(1, "C"), row(2, "C"), row(4, "make"));
    let verify = |tree: &Compilation, target: Option<i64>| {
        let info = tree.canonical_type_info(method).unwrap();
        let Type::Indirect {
            kind: Indirection::Reference(region),
            inner,
            ..
        } = tree.arena.get(info.receiver_type_id.unwrap())
        else {
            panic!("receiver reference");
        };
        assert_eq!(
            super::super::super::head_decl::head_decl_id(&tree.arena, inner),
            target
        );
        assert_eq!(info.parameter_type_ids.as_ref().unwrap().len(), 1);
        let Type::Indirect {
            kind: Indirection::Reference(other),
            ..
        } = tree.arena.get(info.parameter_type_ids.as_ref().unwrap()[0])
        else {
            panic!("ordinary reference");
        };
        assert_ne!(region, other);
        assert_ne!(region, Lifetime::Unknown);
        let Type::Indirect {
            kind: Indirection::Reference(output),
            inner: result,
            ..
        } = tree.arena.get(info.return_type_id.unwrap())
        else {
            panic!("output reference");
        };
        assert_eq!(region, output);
        assert_eq!(inner, result);
        // The unchanged caller's borrow occurrence keeps its identity while
        // its nominal receiver retargets through a provider edit.
        use crate::indexer::resolve::engine::{
            contract::{FileContext, RefContext},
            file_lookup::FileLookup,
            semantic_model::{SemanticModel, SolveOutcome},
        };
        let caller = &parsed[5];
        let reference = caller
            .refs
            .iter()
            .find(|r| r.kind == crate::types::EdgeKind::Calls)
            .unwrap();
        let context = RefContext {
            extracted_ref: reference,
            source_symbol: &caller.symbols[reference.source_symbol_index],
            scope_chain: vec![],
            file_package_id: None,
            source_symbol_id: ids.row_id(&caller.path, reference.source_symbol_index),
        };
        let file = FileContext {
            file_path: caller.path.clone(),
            language: "rust".into(),
            imports: vec![],
            file_namespace: None,
        };
        let lookup = FileLookup::for_file(tree, caller, &ids);
        let result = SemanticModel::production().get_symbol_info(
            &context,
            &file,
            &lookup,
            &crate::languages::rust_lang::profile::RUST_PROFILE,
        );
        if let Some(target) = target {
            let SolveOutcome::Resolved(result) = result else {
                panic!("owned call must bind after provider supply");
            };
            assert_eq!(result.target_symbol_id, method);
            let Type::Indirect { kind, inner, .. } =
                tree.arena.get(result.resolved_yield_type.unwrap())
            else {
                panic!("bound output reference");
            };
            let selector = reference
                .chain
                .as_ref()
                .unwrap()
                .segments
                .last()
                .unwrap()
                .byte_offset;
            assert_eq!(
                kind,
                Indirection::Reference(Lifetime::Inference {
                    owner: row(5, "f"),
                    byte: selector
                })
            );
            assert_eq!(
                super::super::super::head_decl::head_decl_id(&tree.arena, inner),
                Some(target)
            );
        } else {
            assert!(
                !matches!(result, SolveOutcome::Resolved(_)),
                "missing provider cannot leave a stale call target"
            );
        }
        region
    };
    let mut tree = Compilation::build_with_context(
        &parsed[..3],
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    tree.ingest(&parsed[4..], &ids, &Default::default());
    verify(&tree, None);
    tree.ingest(&parsed[3..4], &ids, &Default::default());
    let original = verify(&tree, Some(a));
    tree.persist_type_info(db.conn()).unwrap();
    for (source, target) in [("pub use crate::b::C;", b), ("pub use crate::a::C;", a)] {
        let changed = parse(&arena, &[("provider.rs", source)]);
        let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &changed,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let mut incremental = Compilation::build_with_context(
            &changed,
            &changed_ids,
            Arc::clone(&arena),
            Some(&context),
            &Default::default(),
        );
        incremental.ingest_from_db(db.conn());
        assert_eq!(verify(&incremental, Some(target)), original);
        incremental.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        let snapshot: String = db
            .conn()
            .query_row(
                "SELECT value FROM _bearwisdom_meta WHERE key='type_arena_snapshot'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        restored.restore_snapshot(&snapshot);
        let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
        cold.ingest_from_db(db.conn());
        assert_eq!(verify(&cold, Some(target)), original);
    }
    db.conn()
        .execute("DELETE FROM files WHERE path='provider.rs'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    deleted.ingest_from_db(db.conn());
    verify(&deleted, None);
    assert!(deleted.member_pattern(method).is_none());
}
