use super::super::super::module_trait_inputs::{Data, Owner};
use super::super::super::module_type_inputs::Recipe;
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
                crate::languages::default_registry(),
                arena,
            )
            .unwrap()
        })
        .collect()
}

fn materialize(tree: &Compilation, file: &str, recipe: &Recipe) -> TypeId {
    recipe.materialize_with_context(
        &tree.arena,
        &|binding| {
            Some(
                tree.modules
                    .binding(file, binding, true)
                    .declaration()
                    .and_then(|id| tree.symbol_by_id(id))
                    .map(|symbol| tree.arena.decl(&symbol.qualified_name, symbol.id))
                    .unwrap_or_else(|| tree.arena.intern(Type::Unknown)),
            )
        },
        &|binding, index| {
            tree.modules
                .binding(file, binding, true)
                .declaration()
                .and_then(|id| tree.canonical_type_info(id))
                .and_then(|info| info.generic_param_ids.get(index).copied())
                .map(|id| tree.arena.generic_type(id))
        },
        Some(tree),
    )
}

fn head(tree: &Compilation, file: &str, recipe: &Recipe) -> Option<i64> {
    super::super::super::head_decl::head_decl_id(&tree.arena, materialize(tree, file, recipe))
}

#[test]
fn trait_inputs_rebind_actual_provider_edits_deletion_and_fresh_arena_cold_reload() {
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
        ("a.rs", "pub trait Save { fn save(&self) {} } pub struct Doc;"),
        ("b.rs", "pub trait Save { fn save(&self) {} } pub struct Doc;"),
        ("provider.rs", "pub use crate::a::{Save as Active, Doc as Input};"),
        ("body.rs", "use crate::provider::{Active, Input}; impl Active for Input { fn save(&self) {} } pub fn run(p:&Input) { p.save(); }")]);
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
    let method = row(4, "save");
    let check = |tree: &Compilation, expected: Option<(i64, i64)>| {
        let data = &tree.modules.inputs["body.rs"].traits;
        assert_eq!(data.headers.len(), 1);
        assert_eq!(data.implementations.len(), 1);
        let header = &data.headers[0];
        let implementation = &data.implementations[0];
        assert_eq!(header.owner, Owner::Implementation(implementation.owner));
        assert!(header.enabled);
        assert_eq!(header.members, [method]);
        assert_eq!(
            head(tree, "body.rs", &implementation.trait_type).zip(head(
                tree,
                "body.rs",
                &implementation.receiver
            )),
            expected
        );
        for file in [1, 2] {
            assert!(
                !tree
                    .members_of_id(row(file, "Doc"))
                    .iter()
                    .any(|s| s.id == method),
                "trait impl is not an inherent member"
            );
        }
        let caller = &parsed[4];
        let lookup = super::super::super::file_lookup::FileLookup::for_file(tree, caller, &ids);
        let reference = caller
            .refs
            .iter()
            .find(|r| r.kind == EdgeKind::Calls && r.target_name == "save")
            .unwrap();
        let context = super::super::super::testkit::ref_ctx(
            reference,
            &caller.symbols[reference.source_symbol_index],
            vec![],
        );
        let result = super::super::super::chain::bind_member_access(
            &context,
            &super::super::super::testkit::file_ctx(vec![], None),
            &lookup,
            &crate::languages::rust_lang::RUST_PROFILE,
        );
        match expected {
            Some((trait_id, _)) => {
                let expected_method = tree.trait_graph().definitions[&trait_id]
                    .members
                    .values()
                    .flatten()
                    .copied()
                    .next()
                    .unwrap();
                assert_eq!(
                    result.unwrap().target_symbol_id,
                    expected_method,
                    "static target follows the provider, not the impl body"
                );
            }
            None => assert!(
                result.is_err(),
                "deleted trait provider cannot retain a static target"
            ),
        }
    };
    let tree = Compilation::build_with_context(
        &parsed,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    check(&tree, Some((row(1, "Save"), row(1, "Doc"))));
    let source_inputs = serde_json::to_string(&tree.modules.inputs["body.rs"].traits).unwrap();
    tree.persist_type_info(db.conn()).unwrap();
    let changed = parse(
        &arena,
        &[(
            "provider.rs",
            "pub use crate::b::{Save as Active, Doc as Input};",
        )],
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
    check(&incremental, Some((row(2, "Save"), row(2, "Doc"))));
    assert_eq!(
        serde_json::to_string(&incremental.modules.inputs["body.rs"].traits).unwrap(),
        source_inputs,
        "unchanged implementation retains source identities"
    );
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
    let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
    cold.ingest_from_db(db.conn());
    check(&cold, Some((row(2, "Save"), row(2, "Doc"))));
    assert_eq!(
        serde_json::to_string(&cold.modules.inputs["body.rs"].traits).unwrap(),
        source_inputs
    );
    db.conn()
        .execute("DELETE FROM files WHERE path='b.rs'", [])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), restored);
    deleted.ingest_from_db(db.conn());
    check(&deleted, None);
}

#[test]
fn source_trait_bounds_preserve_generic_parameter_ids_and_namesake_provider_heads() {
    let arena = Arc::new(TypeArena::new());
    let db = crate::Database::open_in_memory().unwrap();
    let parsed = parse(&arena, &[("lib.rs", "mod a { pub trait Save {} } mod b { pub trait Save {} }
        struct Pair<A,B>(A,B); trait Get<T> {} impl<T:a::Save,U> Get<U> for Pair<T,U> where U:b::Save {}
        fn f<T:a::Save>(p:&T) where T:b::Save {}")]);
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(&parsed, &ids, arena);
    let data = &tree.modules.inputs["lib.rs"].traits;
    let row = |qname: &str| {
        ids.row_id(
            "lib.rs",
            parsed[0]
                .symbols
                .iter()
                .position(|s| s.qualified_name == qname)
                .unwrap(),
        )
        .unwrap()
    };
    let implementation = data.implementations.first().unwrap();
    let Recipe::Apply(_, args) = &implementation.receiver else {
        panic!("exact receiver application");
    };
    assert!(
        matches!(args.as_slice(), [Recipe::SourceParameter { binding: a, index: 0 }, Recipe::SourceParameter { binding: b, index: 1 }] if *a == implementation.owner && *b == implementation.owner)
    );
    let function = row("f");
    let parameter = tree
        .canonical_type_info(function)
        .unwrap()
        .generic_param_ids[0];
    let bounds: Vec<_> = data
        .bounds
        .iter()
        .filter(|bound| bound.owner == Owner::Declaration(function))
        .collect();
    assert_eq!(
        bounds.len(),
        2,
        "inline and where clauses are both obligations"
    );
    for bound in &bounds {
        assert_eq!(
            materialize(&tree, "lib.rs", &bound.subject),
            tree.arena.generic_type(parameter)
        );
    }
    assert_eq!(
        bounds
            .iter()
            .map(|bound| head(&tree, "lib.rs", &bound.traits[0]))
            .collect::<Vec<_>>(),
        [Some(row("a.Save")), Some(row("b.Save"))]
    );
    let encoded = serde_json::to_string(data).unwrap();
    let restored: Data = serde_json::from_str(&encoded).unwrap();
    assert_eq!(serde_json::to_string(&restored).unwrap(), encoded);
}
