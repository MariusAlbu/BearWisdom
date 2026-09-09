use super::*;

fn put(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}
fn setup() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "Cargo.toml", "[package]\nname='app'\nversion='0.1.0'\n[dependencies]\napi={package='provider',version='1'}");
    put(dir.path(), "src/lib.rs", "");
    put(dir.path(), "Cargo.lock", &format!("version=3\n[[package]]\nname='app'\nversion='0.1.0'\ndependencies=['provider']\n[[package]]\nname='provider'\nversion='1.2.0'\nsource='registry+https://example.test/index'\nchecksum='{}'", "a".repeat(64)));
    put(
        dir.path(),
        "registry/provider-1.2.0/Cargo.toml",
        "[package]\nname='provider'\nversion='1.2.0'\n[lib]\nname='other_api'\npath='api/root.rs'",
    );
    put(
        dir.path(),
        "registry/provider-1.2.0/.cargo-checksum.json",
        &format!("{{\"package\":\"{}\",\"files\":{{}}}}", "a".repeat(64)),
    );
    put(dir.path(), "registry/provider-1.2.0/api/root.rs", "");
    dir
}
fn load(root: &Path) -> Vec<ModulePackage> {
    let mut entries = vec![module_manifest::entry(root.join("Cargo.toml"), root).unwrap()];
    extend(&mut entries, root, &[root.join("registry")]);
    entries.remove(0).data.module_packages
}
#[test]
fn exact_lock_edge_checksum_manifest_and_custom_library_supply_source_configuration() {
    let dir = setup();
    let records = load(dir.path());
    assert_eq!(records.len(), 2);
    let dep = &records[0].dependencies[0];
    assert_eq!(dep.root.as_deref(), Some("ext:rust:provider"));
    assert!(dep.renamed);
    assert!(!dep.conditional);
    assert_eq!(records[1].root, "ext:rust:provider");
    assert_eq!(records[1].targets[0].name, "other_api");
    assert_eq!(records[1].targets[0].path, "api/root.rs");
}
#[test]
fn stale_manifest_requirement_cannot_borrow_a_locked_namesake() {
    let dir = setup();
    put(dir.path(), "Cargo.toml", "[package]\nname='app'\nversion='0.1.0'\n[dependencies]\napi={package='provider',version='2'}");
    assert!(load(dir.path())[0].dependencies[0].conditional);
}
#[test]
fn checksum_and_installed_manifest_must_match_the_lock_identity() {
    for (path, content) in [
        (".cargo-checksum.json", "{\"package\":\"wrong\"}"),
        ("Cargo.toml", "[package]\nname='provider'\nversion='2.0.0'"),
    ] {
        let dir = setup();
        put(
            dir.path(),
            &format!("registry/provider-1.2.0/{path}"),
            content,
        );
        let records = load(dir.path());
        assert_eq!(records.len(), 1);
        assert!(records[0].dependencies[0].conditional);
    }
}
#[test]
fn missing_owner_lock_edge_cannot_make_a_transitive_package_direct() {
    let dir = setup();
    let path = dir.path().join("Cargo.lock");
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("dependencies=['provider']", "dependencies=[]");
    put(dir.path(), "Cargo.lock", &text);
    assert!(load(dir.path())[0].dependencies[0].conditional);
}
#[test]
fn version_collisions_are_incomplete_until_external_addresses_are_versioned() {
    let dir = setup();
    let text = std::fs::read_to_string(dir.path().join("Cargo.lock")).unwrap();
    put(dir.path(), "Cargo.lock", &format!("{text}\n[[package]]\nname='provider'\nversion='2.0.0'\nsource='registry+https://example.test/index'"));
    let records = load(dir.path());
    assert_eq!(records.len(), 1);
    assert!(records[0].dependencies[0].conditional);
}
#[test]
fn lock_and_provider_configuration_edits_change_fingerprints() {
    let dir = setup();
    let before = load(dir.path());
    put(
        dir.path(),
        "registry/provider-1.2.0/Cargo.toml",
        "[package]\nname='provider'\nversion='1.2.0'\n[lib]\npath='api/new.rs'",
    );
    let after = load(dir.path());
    assert_ne!(before[0].fingerprint, after[0].fingerprint);
    assert_ne!(before[1].fingerprint, after[1].fingerprint);
    let text = std::fs::read_to_string(dir.path().join("Cargo.lock")).unwrap();
    put(dir.path(), "Cargo.lock", &format!("{text}\n# changed\n"));
    assert_ne!(after[0].fingerprint, load(dir.path())[0].fingerprint);
}

#[test]
fn duplicate_install_locations_and_optional_dependencies_do_not_become_certain() {
    let dir = setup();
    let mut entries =
        vec![module_manifest::entry(dir.path().join("Cargo.toml"), dir.path()).unwrap()];
    extend(
        &mut entries,
        dir.path(),
        &[dir.path().join("registry"), dir.path().join("registry")],
    );
    assert!(entries[0].data.module_packages[0].dependencies[0].conditional);
    put(dir.path(), "Cargo.toml", "[package]\nname='app'\nversion='0.1.0'\n[dependencies]\napi={package='provider',version='1',optional=true}");
    assert!(load(dir.path())[0].dependencies[0].conditional);
}

#[test]
fn conflicting_versions_in_separate_workspace_locks_cannot_share_an_external_address() {
    let dir = setup();
    let manifest = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
    let lock = std::fs::read_to_string(dir.path().join("Cargo.lock")).unwrap();
    put(dir.path(), "nested/Cargo.toml", &manifest);
    put(
        dir.path(),
        "nested/Cargo.lock",
        &lock.replace("version='1.2.0'", "version='2.0.0'"),
    );
    let mut entries = vec![
        module_manifest::entry(dir.path().join("Cargo.toml"), dir.path()).unwrap(),
        module_manifest::entry(dir.path().join("nested/Cargo.toml"), dir.path()).unwrap(),
    ];
    extend(&mut entries, dir.path(), &[dir.path().join("registry")]);
    for entry in entries {
        assert_eq!(entry.data.module_packages.len(), 1);
        assert!(entry.data.module_packages[0].dependencies[0].conditional);
    }
}

#[test]
fn named_registry_configuration_is_not_inferred_from_a_stale_lock() {
    let dir = setup();
    put(dir.path(), "Cargo.toml", "[package]\nname='app'\nversion='0.1.0'\n[dependencies]\napi={package='provider',version='1',registry='changed-registry'}");
    assert!(load(dir.path())[0].dependencies[0].conditional);
}

#[test]
fn configured_registry_trait_targets_and_returns_follow_provider_edits_and_cold_reload() {
    use crate::{
        indexer::{
            project_context::ProjectContext,
            resolve::engine::{compilation::Compilation, pipeline::resolve_from_tree},
            symbol_ids::SymbolIds,
            write::write_parsed_files_with_origin,
        },
        type_checker::core::types::TypeArena,
        types::EdgeKind,
        Database,
    };
    use std::sync::Arc;
    let dir = setup();
    let caller = "use api::{Thing, Load}; pub fn run(p:&Thing) { p.load().touch(); }";
    let body = "pub struct Thing; pub struct Doc; impl Doc { pub fn touch(&self) {} } pub trait Load { fn load(&self)->Doc; } impl Load for Thing { fn load(&self)->Doc { Doc } }";
    let provider = |active: &str| {
        format!(
            "pub mod a {{ {body} }} pub mod b {{ {body} }} pub use {active}::{{Thing, Load, Doc}};"
        )
    };
    let provider_path = "ext:rust:provider/api/root.rs";
    let arena = Arc::new(TypeArena::new());
    let mut db = Database::open_in_memory().unwrap();
    let parse = |path: &str, disk: &str, source: &str| {
        put(dir.path(), disk, source);
        crate::indexer::parse_file::parse_file_with_arena(
            &crate::walker::WalkedFile {
                relative_path: path.into(),
                absolute_path: dir.path().join(disk),
                language: "rust",
            },
            crate::languages::default_registry(),
            &arena,
        )
        .unwrap()
    };
    let caller = parse("src/lib.rs", "src/lib.rs", caller);
    let mut provider_file = parse(
        provider_path,
        "registry/provider-1.2.0/api/root.rs",
        &provider("a"),
    );
    crate::indexer::contract_filter::reduce_to_contract(&mut provider_file);
    let (_, mut ids) = write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&caller),
        "internal",
        Some(&arena),
    )
    .unwrap();
    let (_, external_ids) = write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&provider_file),
        "external",
        Some(&arena),
    )
    .unwrap();
    ids.merge(external_ids);
    let mut context = ProjectContext::default();
    context.manifests.insert(
        crate::ecosystem::manifest::ManifestKind::Cargo,
        crate::ecosystem::manifest::ManifestData {
            module_packages: load(dir.path()),
            ..Default::default()
        },
    );
    let parsed = [caller, provider_file];
    let caller = &parsed[0];
    let provider_file = &parsed[1];
    let tree = Compilation::build_with_context(
        &parsed,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    let check = |tree: Compilation,
                 db: &mut Database,
                 ids: &SymbolIds,
                 provider_file: &crate::types::ParsedFile,
                 active: Option<&str>| {
        resolve_from_tree(db, tree, std::slice::from_ref(caller), ids, None).unwrap();
        let references: Vec<_> = caller
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .collect();
        assert_eq!(references.len(), 2);
        for reference in references {
            let expected = active.map(|active| {
                let qname = match reference.target_name.as_str() {
                    "load" => format!("{active}.Load.load"),
                    "touch" => format!("{active}.Doc.touch"),
                    _ => panic!("unexpected reference"),
                };
                let slot = provider_file
                    .symbols
                    .iter()
                    .position(|s| s.qualified_name == qname)
                    .unwrap();
                ids.row_id(provider_path, slot).unwrap()
            });
            let actual: Option<i64> = db.conn().query_row("SELECT target_id FROM ref_resolutions WHERE source_id=?1 AND source_selector_byte=?2 AND kind='calls'",
                rusqlite::params![ids.row_id(&caller.path, reference.source_symbol_index).unwrap(), reference.chain.as_ref().unwrap().segments.last().unwrap().byte_offset], |r| r.get(0)).unwrap();
            assert_eq!(
                actual, expected,
                "{}: exact static declaration, not the implementation or namesake",
                reference.target_name
            );
        }
    };
    check(tree, &mut db, &ids, provider_file, Some("a"));
    let mut changed = parse(
        provider_path,
        "registry/provider-1.2.0/api/root.rs",
        &provider("b"),
    );
    crate::indexer::contract_filter::reduce_to_contract(&mut changed);
    let (_, changed_ids) = write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&changed),
        "external",
        Some(&arena),
    )
    .unwrap();
    ids.merge(changed_ids);
    let mut edited = Compilation::build_with_context(
        std::slice::from_ref(&changed),
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    edited.ingest_from_db(db.conn());
    check(edited, &mut db, &ids, &changed, Some("b"));
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
    check(cold, &mut db, &ids, &changed, Some("b"));
    db.conn()
        .execute("DELETE FROM files WHERE path=?1", [provider_path])
        .unwrap();
    let mut deleted = Compilation::build(&[], &SymbolIds::default(), restored);
    deleted.ingest_from_db(db.conn());
    check(deleted, &mut db, &ids, &changed, None);
}
