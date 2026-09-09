use super::*;

#[path = "namespace_place_tests.rs"]
mod places;

fn table(source: &str) -> Table {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    fn walk(node: Node, source: &[u8], table: &mut Table) {
        capture(
            node,
            source,
            &crate::languages::rust_lang::namespaces::FORMS,
            table,
        );
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            walk(child, source, table);
        }
    }
    let mut result = Table::default();
    walk(tree.root_node(), source.as_bytes(), &mut result);
    result
}

#[test]
fn bare_member_qualified_and_grouped_calls_have_exact_source_operands() {
    let source = "fn run(p:C) { same(&p); p.same(&mut p); C::same::<C>(&p); <C as T>::same(&p); (same)(&p); same(); }";
    let table = table(source);
    assert_eq!(table.len(), 6);
    for (index, (byte, _)) in source.match_indices("same").enumerate() {
        let arguments = table[&(byte as u32)].as_ref().unwrap();
        assert_eq!(arguments.len(), usize::from(index != 5));
        if index != 5 {
            let CallArg::BorrowAt { span, expr } = &arguments[0] else {
                panic!("borrow shape");
            };
            assert!(source[span.start as usize..span.end as usize].starts_with('&'));
            assert!(matches!(expr.as_ref(), CallArg::IdentAt(_)));
        }
    }
}

#[test]
fn malformed_or_shared_selector_calls_are_authoritative_unknown_not_first_wins() {
    let source = "fn run(p:C) { factory()(&p); target(&); }";
    let table = table(source);
    assert_eq!(
        table.get(&(source.find("factory()").unwrap() as u32)),
        Some(&None)
    );
    assert_eq!(
        table.get(&(source.find("target(").unwrap() as u32)),
        Some(&None)
    );
}

#[test]
fn source_call_local_cascades_follow_provider_signature_edits_deletion_and_cold_reload() {
    use crate::type_checker::core::types::TypeArena;
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
    for (filtered, borrowed_local) in [(false, false), (true, false), (false, true), (true, true)] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname='sample'\nversion='0.0.0'\nedition='2021'\n[lib]\npath='lib.rs'",
        )
        .unwrap();
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
        let mut caller = parse(
            "lib.rs",
            if borrowed_local {
                "mod api; use api::keep as hold; pub fn run(p:api::Input) { let r=&p; let x=hold(r); x.touch(); let y=api::keep(r); y.touch(); }"
            } else {
                "mod api; use api::keep as hold; pub fn run(p:api::Input) { let x=hold(&p); x.touch(); let y=api::keep(&p); y.touch(); }"
            },
        );
        for reference in &mut caller.refs {
            reference.call_args.clear();
            if let Some(chain) = &mut reference.chain {
                for segment in &mut chain.segments {
                    segment.call_args =
                        vec![crate::types::CallArg::Ident("wrong_display_operand".into())];
                }
            }
        }
        let provider = |active: &str| {
            let mut file = parse("api.rs", &format!("pub struct Input; pub mod a {{ pub struct Doc; impl Doc {{ pub fn touch(&self) {{}} }} }}
                pub mod b {{ pub struct Doc; impl Doc {{ pub fn touch(&self) {{}} }} }} pub fn keep<'a>(p:&'a Input)->&'a {active}::Doc {{ loop {{}} }}"));
            if filtered {
                crate::indexer::contract_filter::reduce_to_contract(&mut file);
            }
            file
        };
        let parsed = [caller, provider("a")];
        let caller = &parsed[0];
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
                     file: &crate::types::ParsedFile,
                     active: Option<&str>| {
            resolve_from_tree(db, tree, std::slice::from_ref(caller), ids, None).unwrap();
            let references: Vec<_> = caller
                .refs
                .iter()
                .filter(|r| r.kind == EdgeKind::Calls)
                .collect();
            assert_eq!(references.len(), 4);
            for reference in references {
                let expected = active.map(|active| {
                    let qname = if reference.target_name == "touch" {
                        format!("{active}.Doc.touch")
                    } else {
                        "keep".into()
                    };
                    ids.row_id(
                        "api.rs",
                        file.symbols
                            .iter()
                            .position(|s| s.qualified_name == qname)
                            .unwrap(),
                    )
                    .unwrap()
                });
                let byte = reference
                    .chain
                    .as_ref()
                    .and_then(|c| c.segments.last())
                    .map(|s| s.byte_offset)
                    .unwrap_or(reference.byte_offset);
                let actual: Option<i64> = db.conn().query_row("SELECT target_id FROM ref_resolutions WHERE source_id=?1 AND source_selector_byte=?2 AND kind='calls'",
                    rusqlite::params![ids.row_id(&caller.path, reference.source_symbol_index).unwrap(), byte], |r| r.get(0)).unwrap();
                assert_eq!(
                    actual, expected,
                    "{}: filtered={filtered}, active={active:?}",
                    reference.target_name
                );
            }
        };
        check(tree, &mut db, &ids, &parsed[1], Some("a"));
        let changed = provider("b");
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
        db.conn()
            .execute("DELETE FROM files WHERE path='api.rs'", [])
            .unwrap();
        let mut deleted = Compilation::build(&[], &SymbolIds::default(), restored);
        deleted.ingest_from_db(db.conn());
        check(deleted, &mut db, &ids, &changed, None);
    }
}
