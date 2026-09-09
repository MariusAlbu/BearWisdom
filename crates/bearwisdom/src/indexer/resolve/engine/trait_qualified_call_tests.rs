use super::*;
use crate::indexer::resolve::engine::{
    member_index::MemberIndex,
    testkit::{sym, Lookup},
};

#[test]
fn missing_explicit_trait_identity_cannot_select_a_namesake() {
    let lookup = Lookup::new().with(sym(3, "save", "Doc.save", "method", "lib.rs"));
    let arena = TypeArena::new();
    let call = QualifiedCall {
        caller: Some(1),
        obligation: Obligation {
            subject: arena.decl("Doc", 2),
            trait_type: arena.intern(Type::Unknown),
        },
    };
    let mut names = MemberIndex::default();
    names.record(2, "save", 3);
    assert!(select(
        &Graph::default(),
        &lookup,
        &arena,
        &call,
        names.name("save").unwrap(),
        &[],
        &[]
    )
    .is_err());
}

#[test]
fn qualified_targets_and_returns_follow_provider_edits_filtering_deletion_and_cold_reload() {
    use crate::{
        indexer::{
            resolve::engine::{compilation::Compilation, pipeline::resolve_from_tree},
            symbol_ids::SymbolIds,
            write::write_parsed_files_with_origin,
        },
        types::EdgeKind,
        Database,
    };
    use std::sync::Arc;
    for (filtered, borrowed) in [(false, false), (true, false), (false, true), (true, true)] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname='sample'\nversion='0.0.0'\nedition='2021'\n[lib]\npath='lib.rs'",
        )
        .unwrap();
        let body = "pub struct Input; pub struct Doc; impl Doc { pub fn touch(&self) {} } pub trait Load { fn load(&self)->Doc; } impl Load for Input { fn load(&self)->Doc { Doc } }";
        let provider = |active| {
            format!(
                "pub mod a {{ {body} }} pub mod b {{ {body} }} pub use {active}::{{Input,Load}};"
            )
        };
        let arena = Arc::new(TypeArena::new());
        let mut db = Database::open_in_memory().unwrap();
        let parse = |path: &str, source: &str| {
            std::fs::write(dir.path().join(path), source).unwrap();
            crate::indexer::parse_file::parse_file_with_arena(
                &crate::walker::WalkedFile {
                    relative_path: path.into(),
                    absolute_path: dir.path().join(path),
                    language: "rust",
                },
                crate::languages::default_registry(),
                &arena,
            )
            .unwrap()
        };
        let caller = parse(
            "lib.rs",
            if borrowed {
                "mod api; pub fn run(p:api::Input) { <api::Input as api::Load>::load(&p).touch(); }"
            } else {
                "mod api; pub fn run(p:&api::Input) { <api::Input as api::Load>::load(p).touch(); }"
            },
        );
        let mut provider_file = parse("api.rs", &provider("a"));
        if filtered {
            crate::indexer::contract_filter::reduce_to_contract(&mut provider_file);
        }
        let parsed = [caller, provider_file];
        let caller = &parsed[0];
        let provider_file = &parsed[1];
        let (_, mut ids) =
            write_parsed_files_with_origin(&db, &parsed, "internal", Some(&arena)).unwrap();
        let context = crate::indexer::project_context::build_project_context(dir.path());
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
                    ids.row_id("api.rs", slot).unwrap()
                });
                let actual: Option<i64> = db.conn().query_row("SELECT target_id FROM ref_resolutions WHERE source_id=?1 AND source_selector_byte=?2 AND kind='calls'",
                    rusqlite::params![ids.row_id(&caller.path, reference.source_symbol_index).unwrap(), reference.chain.as_ref().unwrap().segments.last().unwrap().byte_offset], |r| r.get(0)).unwrap();
                assert_eq!(actual, expected, "{}: exact static declaration, not implementation/namesake; filtered={filtered}", reference.target_name);
            }
        };
        check(tree, &mut db, &ids, provider_file, Some("a"));
        let mut changed = parse("api.rs", &provider("b"));
        if filtered {
            crate::indexer::contract_filter::reduce_to_contract(&mut changed);
        }
        let (_, changed_ids) = write_parsed_files_with_origin(
            &db,
            std::slice::from_ref(&changed),
            "internal",
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
        let without_impl = provider("b").replace(
            "impl Load for Input { fn load(&self)->Doc { Doc } }",
            "impl Input { pub fn load(&self)->Doc { Doc } }",
        );
        let mut missing_impl = parse("api.rs", &without_impl);
        if filtered {
            crate::indexer::contract_filter::reduce_to_contract(&mut missing_impl);
        }
        let (_, changed_ids) = write_parsed_files_with_origin(
            &db,
            std::slice::from_ref(&missing_impl),
            "internal",
            Some(&arena),
        )
        .unwrap();
        ids.merge(changed_ids);
        let mut missing = Compilation::build_with_context(
            std::slice::from_ref(&missing_impl),
            &ids,
            Arc::clone(&arena),
            Some(&context),
            &Default::default(),
        );
        missing.ingest_from_db(db.conn());
        check(missing, &mut db, &ids, &missing_impl, None);
        db.conn()
            .execute("DELETE FROM files WHERE path='api.rs'", [])
            .unwrap();
        let mut deleted = Compilation::build(&[], &SymbolIds::default(), restored);
        deleted.ingest_from_db(db.conn());
        check(deleted, &mut db, &ids, &changed, None);
    }
}
