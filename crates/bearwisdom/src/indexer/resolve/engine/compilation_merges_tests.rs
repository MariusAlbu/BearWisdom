use super::*;

#[test]
fn absent_physical_rows_cannot_attest_a_merge_group() {
    let arena = Arc::new(TypeArena::new());
    let mut tree = Compilation::build(&[], &SymbolIds::default(), arena);
    tree.attest_scoped_merges(&[vec![7, 8], vec![]]);
    assert!(tree.merge_groups.attested.is_empty());
    assert!(tree.merge_groups.scoped.is_empty());
    tree.by_id.insert(
        7,
        crate::indexer::resolve::engine::testkit::sym(7, "Model", "Model", "interface", "a.ts"),
    );
    tree.attest_scoped_merges(&[vec![7, 8]]);
    assert!(
        tree.merge_groups.attested.contains(&7),
        "live rows remain fenced from legacy name merging"
    );
    assert!(
        tree.merge_groups.scoped.is_empty(),
        "a partial group does not attest a canonical target"
    );
}

#[test]
fn bound_rust_constructor_chain_keeps_struct_and_impl_member_identity() {
    assert_rust_constructor_chain("pub struct Builder; impl Builder { pub fn new() -> Builder { Builder } pub fn build(&self) -> Widget { Widget } } pub struct Widget; impl Widget { pub fn show(&self) {} } pub fn f() { Builder::new().build().show(); }");
}

#[test]
fn rust_import_names_and_function_local_impl_owners_survive_cold_reload() {
    for import in [
        "use foreign::Other;",
        "use foreign::Builder as Other;",
        "use foreign::{Other, nested::{Item as Renamed}};",
        "use foreign::*;",
    ] {
        // Previously foreign did not exist: the fixture assumed a guessed call
        // target under an unknown trait provider. Keep ownership unconditional,
        // and make named-provider call evidence real. Opaque glob stays unknown.
        assert_rust_constructor_chain_mode(&format!("mod foreign {{ pub struct Other; pub struct Builder; pub mod nested {{ pub struct Item; }} }} pub fn f() {{ {import} struct Builder; impl Builder {{ fn new() -> Builder {{ Builder }} fn build(&self) -> Widget {{ Widget }} }} struct Widget; impl Widget {{ fn show(&self) {{}} }} Builder::new().build().show(); }}"), !import.contains('*'));
    }
}

fn assert_rust_constructor_chain(source: &str) {
    assert_rust_constructor_chain_mode(source, true);
}

fn assert_rust_constructor_chain_mode(source: &str, bind: bool) {
    let arena = Arc::new(TypeArena::new());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.rs");
    std::fs::write(&path, source).unwrap();
    let parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "a.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        &crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&parsed),
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(std::slice::from_ref(&parsed), &ids, Arc::clone(&arena));
    let expected = parsed
        .symbols
        .iter()
        .position(|s| s.kind == SymbolKind::Method && s.name == "show")
        .and_then(|slot| ids.row_id("a.rs", slot))
        .unwrap();
    tree.persist_type_info(db.conn()).unwrap();
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
    let refs: Vec<_> = parsed
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls && r.target_name == "show")
        .collect();
    assert_eq!(refs.len(), 1);
    for tree in [&tree, &cold] {
        let owner = parsed
            .symbols
            .iter()
            .position(|s| s.name == "Widget" && s.kind == SymbolKind::Struct)
            .unwrap();
        assert!(
            tree.members_of_id(ids.row_id("a.rs", owner).unwrap())
                .iter()
                .any(|s| s.id == expected),
            "lexical impl ownership survives even when trait availability is incomplete"
        );
        let lookup =
            crate::indexer::resolve::engine::file_lookup::FileLookup::for_file(tree, &parsed, &ids);
        for reference in &refs {
            let source = &parsed.symbols[reference.source_symbol_index];
            let mut context =
                crate::indexer::resolve::engine::testkit::ref_ctx(reference, source, vec![]);
            context.source_symbol_id = ids.row_id("a.rs", reference.source_symbol_index);
            let result = crate::indexer::resolve::engine::chain::bind_member_access(
                &context,
                &crate::indexer::resolve::engine::testkit::file_ctx(vec![], None),
                &lookup,
                &crate::languages::rust_lang::profile::RUST_PROFILE,
            );
            if bind {
                assert_eq!(
                    result
                        .unwrap_or_else(|cause| panic!("{reference:?}: {cause:?}"))
                        .target_symbol_id,
                    expected
                );
            } else {
                assert!(
                    result.is_err(),
                    "opaque glob must not certify method precedence"
                );
            }
        }
    }
}
