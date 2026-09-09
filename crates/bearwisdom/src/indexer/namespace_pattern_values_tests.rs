use super::*;

#[test]
fn pattern_syntax_preserves_constructor_fields_and_guard_boundary() {
    let source = "fn f(x:E) { match x { E::Both { first, r#type: second } if first.ok() => second.save() } }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    let forms = &crate::languages::rust_lang::namespaces::FORMS;
    let mut symbols = Vec::new();
    let mut data = crate::indexer::namespaces::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &symbols,
        &[],
    )
    .unwrap();
    let graph = crate::indexer::namespaces::capture_locals(
        &mut data,
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &mut symbols,
        &[],
        crate::indexer::flow::BindingSymbols::Synthesize,
    );
    for spelling in ["first", "second"] {
        let binding = graph
            .binding_at(
                source.rfind(spelling).unwrap() as u32,
                graph.name_id(spelling).unwrap(),
            )
            .unwrap();
        assert!(
            matches!(
                graph.types.values.get(&binding),
                Some(ValueExpr::VariantField { .. })
            ),
            "{spelling}: {:?}; {}",
            graph.types.values,
            tree.root_node().to_sexp()
        );
    }
    assert_eq!(graph.types.pattern_heads.len(), 1);
    assert_eq!(forms.patterns.variant_declaration, "enum_variant");
}

#[test]
fn pattern_payload_cascades_follow_provider_edits_access_removal_and_cold_reload() {
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
    for filtered in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='pattern_edits'\nversion='0.0.0'\nedition='2021'\n[lib]\npath='lib.rs'").unwrap();
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
        let mut caller = parse("lib.rs", "mod api; use api::E as Choice; pub fn run(p:Choice) { match p { Choice::Item(value) => { let x=value.next(); x.touch(); }, Choice::Named { item:value } => { value.next().touch(); } } }");
        for reference in &mut caller.refs {
            reference.call_args.clear();
            if let Some(chain) = &mut reference.chain {
                for segment in &mut chain.segments {
                    segment.call_args = vec![crate::types::CallArg::Ident("poison".into())];
                }
            }
        }
        let provider = |active: &str, public: bool, payload: bool| {
            let variants = if payload {
                format!("Item({active}::Doc), Named {{ item:{active}::Doc }}")
            } else {
                "Item(), Named {}".into()
            };
            let visibility = if public { "pub" } else { "" };
            let mut file = parse("api.rs", &format!("pub mod a {{ pub struct Doc; impl Doc {{ pub fn next(&self)->&Self {{self}} pub fn touch(&self) {{}} }} }}
                pub mod b {{ pub struct Doc; impl Doc {{ pub fn next(&self)->&Self {{self}} pub fn touch(&self) {{}} }} }} {visibility} enum E {{ {variants} }}"));
            if filtered {
                crate::indexer::contract_filter::reduce_to_contract(&mut file);
            }
            file
        };
        let files = [caller, provider("a", true, true)];
        let caller = &files[0];
        let (_, mut ids) =
            write_parsed_files_with_origin(&db, &files, "internal", Some(&arena)).unwrap();
        let context = crate::indexer::project_context::build_project_context(dir.path());
        let check = |tree: Compilation,
                     db: &mut Database,
                     ids: &SymbolIds,
                     provider: Option<&crate::types::ParsedFile>,
                     active: Option<&str>| {
            resolve_from_tree(db, tree, std::slice::from_ref(caller), ids, None).unwrap();
            let refs: Vec<_> = caller
                .refs
                .iter()
                .filter(|r| r.kind == EdgeKind::Calls)
                .collect();
            assert_eq!(refs.len(), 4);
            for reference in refs {
                let expected = active.zip(provider).and_then(|(active, file)| {
                    ids.row_id(
                        "api.rs",
                        file.symbols
                            .iter()
                            .position(|s| {
                                s.qualified_name
                                    == format!("{active}.Doc.{}", reference.target_name)
                            })
                            .unwrap(),
                    )
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
        check(
            Compilation::build_with_context(
                &files,
                &ids,
                Arc::clone(&arena),
                Some(&context),
                &Default::default(),
            ),
            &mut db,
            &ids,
            Some(&files[1]),
            Some("a"),
        );
        for (public, payload) in [(true, true), (true, false), (false, true)] {
            let changed = provider("b", public, payload);
            let (_, new_ids) = write_parsed_files_with_origin(
                &db,
                std::slice::from_ref(&changed),
                "internal",
                Some(&arena),
            )
            .unwrap();
            ids.merge(new_ids);
            let mut tree = Compilation::build_with_context(
                std::slice::from_ref(&changed),
                &ids,
                Arc::clone(&arena),
                Some(&context),
                &Default::default(),
            );
            tree.ingest_from_db(db.conn());
            let active = (public && payload).then_some("b");
            check(tree, &mut db, &ids, Some(&changed), active);
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
            check(cold, &mut db, &ids, Some(&changed), active);
        }
        db.conn()
            .execute("DELETE FROM files WHERE path='api.rs'", [])
            .unwrap();
        let mut deleted = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
        deleted.ingest_from_db(db.conn());
        check(deleted, &mut db, &ids, None, None);
    }
}
