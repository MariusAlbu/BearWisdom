use super::*;
use crate::indexer::lexical::LexicalBindings;
use crate::indexer::resolve::engine::cause::{Cause, CauseKind};
use std::sync::Arc;

#[test]
fn pattern_payload_types_preserve_nominal_identity_and_reject_unproved_binding_modes() {
    use crate::type_checker::core::types::Type;
    let source="pub struct Doc; pub struct Other; pub enum E<T> { Item(T) } pub enum F<T> { Item(T) } type Fixed=E<Other>;
        fn valid(x:E<Doc>) { match x { E::Item(payload) => { payload; } } }
        fn wrong(x:F<Doc>) { match x { E::Item(wrong) => { wrong; } } }
        fn borrowed(x:&E<Doc>) { match x { E::Item(borrowed) => { borrowed; } } }
        fn raw(x:*const E<Doc>) { match x { E::Item(raw) => { raw; } } }
        fn fixed(x:E<Doc>) { match x { Fixed::Item(fixed) => { fixed; } } }
        fn explicit_ref(x:E<Doc>) { match x { E::Item(ref explicit) => { explicit; } } }
        fn rest(x:E<Doc>) { match x { E::Item(.., rest) => { rest; } } }";
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='pattern_types'\nversion='0.0.0'\nedition='2021'\n[lib]\npath='lib.rs'",
    )
    .unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, source).unwrap();
    let arena = Arc::new(TypeArena::new());
    let file = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "lib.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let files = [file];
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let context = crate::indexer::project_context::build_project_context(dir.path());
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    let lookup = FileLookup::for_file(&tree, &files[0], &ids);
    let doc = ids
        .row_id(
            "lib.rs",
            files[0]
                .symbols
                .iter()
                .position(|s| s.qualified_name == "Doc")
                .unwrap(),
        )
        .unwrap();
    lookup.set_cursor(source.find("payload;").unwrap() as u32);
    assert!(
        matches!(arena.get(lookup.local_type_id("payload").unwrap()),Type::Decl{symbol_id,..} if symbol_id==doc)
    );
    for name in ["wrong", "borrowed", "raw", "fixed", "explicit", "rest"] {
        lookup.set_cursor(source.find(&format!("{name};")).unwrap() as u32);
        assert_eq!(
            lookup.local_type_id(name),
            Some(arena.intern(Type::Unknown)),
            "{name}"
        );
    }
}

#[test]
fn source_place_types_preserve_regions_privacy_and_position_without_changing_query_cursor() {
    use crate::type_checker::core::types::{Indirection, Type};
    use crate::types::SourceSpan;
    let source="pub struct Doc; pub struct Holder<'a> { pub value:&'a Doc } pub fn outside(p:api::Secret,q:Holder<'_>,t:&(Doc,Holder<'_>),raw:*const Doc) {
        let denied=p.hidden; let allowed=p.visible; let value=q.value; let deref=*t; let second=t.1; let out=t.4; let pointer=raw.value;
        let explicit=&*raw; let absent=p.missing; let not_field=p.method; }
        pub mod api { pub struct Secret { hidden:super::Doc,pub visible:super::Doc } impl Secret { pub fn method(&self)->super::Doc {loop {}} }
        pub fn inside(p:Secret) { let internal=p.hidden; } }";
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='place_types'\nversion='0.0.0'\nedition='2021'\n[lib]\npath='lib.rs'",
    )
    .unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, source).unwrap();
    let arena = Arc::new(TypeArena::new());
    let file = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "lib.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let files = [file];
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let context = crate::indexer::project_context::build_project_context(dir.path());
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    let lookup = FileLookup::for_file(&tree, &files[0], &ids);
    let doc = ids
        .row_id(
            "lib.rs",
            files[0]
                .symbols
                .iter()
                .position(|s| s.qualified_name == "Doc")
                .unwrap(),
        )
        .unwrap();
    let private = ids
        .row_id(
            "lib.rs",
            files[0]
                .symbols
                .iter()
                .position(|s| s.qualified_name == "api.Secret.hidden")
                .unwrap(),
        )
        .unwrap();
    let expr = |text: &str, last: bool| {
        let byte = if last {
            source.rfind(text)
        } else {
            source.find(text)
        }
        .unwrap() as u32;
        SourceSpan {
            start: byte,
            end: byte + text.len() as u32,
        }
    };
    lookup.set_cursor(source.find("let denied").unwrap() as u32);
    assert!(!lookup.declaration_accessible(private));
    assert!(
        matches!(arena.get(lookup.value_expression(expr("p.hidden",true)).unwrap()),Type::Decl {symbol_id,..} if symbol_id==doc)
    );
    assert!(
        !lookup.declaration_accessible(private),
        "expression access must not move active module"
    );
    for text in ["p.hidden", "t.4", "raw.value", "p.missing", "p.method"] {
        assert_eq!(
            lookup.value_expression(expr(text, false)),
            Some(arena.intern(Type::Unknown)),
            "{text}"
        );
    }
    assert!(
        matches!(arena.get(lookup.value_expression(expr("p.visible",false)).unwrap()),Type::Decl {symbol_id,..} if symbol_id==doc)
    );
    let value = lookup.value_expression(expr("q.value", false)).unwrap();
    let Type::Indirect {
        kind: Indirection::Reference(region),
        inner,
        ..
    } = arena.get(value)
    else {
        panic!("reference field");
    };
    assert_ne!(region, crate::type_checker::core::types::Lifetime::Unknown);
    assert!(matches!(arena.get(inner),Type::Decl {symbol_id,..} if symbol_id==doc));
    assert!(
        matches!(arena.get(lookup.value_expression(expr("*t",false)).unwrap()),Type::Tuple(items) if items.len()==2)
    );
    let raw = lookup.value_expression(expr("*raw", false)).unwrap();
    assert!(matches!(arena.get(raw),Type::Decl {symbol_id,..} if symbol_id==doc));
    lookup.set_cursor(source.find("let explicit").unwrap() as u32);
    let borrow = expr("&*raw", false);
    assert_eq!(lookup.local_type_id("value"), Some(value));
    lookup.set_cursor(source.find("let absent").unwrap() as u32);
    assert_eq!(
        lookup.local_type_id("explicit"),
        lookup.borrow_argument(borrow, raw)
    );
    assert_eq!(
        lookup.value_expression(SourceSpan {
            start: borrow.start,
            end: borrow.end + 1
        }),
        None,
        "nearby source spans are not interchangeable"
    );
}

#[test]
fn local_borrow_annotation_keeps_nominal_identity_and_uses_the_initializer_region() {
    use crate::types::SourceSpan;
    let source = "pub mod a { pub struct Doc; } pub mod b { pub struct Doc; } pub fn f(p:a::Doc) { let r:&a::Doc=&p; }";
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='local_value'\nversion='0.0.0'\nedition='2021'\n[lib]\npath='lib.rs'",
    )
    .unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, source).unwrap();
    let arena = Arc::new(TypeArena::new());
    let parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "lib.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let files = [parsed];
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let context = crate::indexer::project_context::build_project_context(dir.path());
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    let lookup = FileLookup::for_file(&tree, &files[0], &ids);
    lookup.set_cursor(source.rfind('}').unwrap() as u32 - 1);
    let operand = lookup.local_type_id("p").unwrap();
    let span = SourceSpan {
        start: source.rfind("&p").unwrap() as u32,
        end: source.rfind("&p").unwrap() as u32 + 2,
    };
    let expected = lookup.borrow_argument(span, operand).unwrap();
    let actual = lookup.local_type_id("r").unwrap();
    assert_eq!(
        actual,
        expected,
        "actual={:?}; expected={:?}; operand={:?}",
        arena.get(actual),
        arena.get(expected),
        arena.get(operand)
    );
}

#[test]
fn source_call_operands_distinguish_empty_missing_and_unmigrated_without_resurrecting_yields() {
    use crate::indexer::resolve::engine::{arg_types, chain, testkit::sym};
    use crate::type_checker::core::types::Type;
    use crate::types::{CallArg, SourceSpan};
    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), Arc::clone(&arena));
    let mut lookup = FileLookup::new(&tree, "rust");
    let legacy = vec![CallArg::Ident("display_only".into())];
    assert_eq!(lookup.source_call_arguments(10), None);
    assert_eq!(arg_types::at(&lookup, 10, &legacy), Some(legacy.as_slice()));
    let source = vec![CallArg::IdentAt(SourceSpan { start: 22, end: 23 })];
    let table = [(10, Some(vec![])), (20, Some(source.clone())), (30, None)]
        .into_iter()
        .collect();
    lookup.call_arguments = Some(&table);
    assert_eq!(lookup.source_call_arguments(10), Some(Ok([].as_slice())));
    assert_eq!(arg_types::at(&lookup, 20, &legacy), Some(source.as_slice()));
    let callee = sym(71, "keep", "keep", "function", "lib.rs");
    let old_yield = arena.decl("stale_display", 99);
    for byte in [10, 20, 30, 40] {
        if byte >= 30 {
            assert_eq!(lookup.source_call_arguments(byte), Some(Err(())));
            assert_eq!(arg_types::at(&lookup, byte, &legacy), None);
        }
        assert_eq!(
            chain::apply_call_args(
                &lookup,
                &arena,
                &callee,
                byte,
                &legacy,
                &[],
                arena.intern(Type::Unknown),
                None,
                Some(old_yield),
                &[]
            ),
            None,
            "captured missing operands or missing signature IDs cannot reuse a previous yield"
        );
    }
}

#[test]
fn source_call_return_ids_equal_the_actual_borrow_for_bare_member_associated_and_namespace_calls() {
    use crate::indexer::resolve::engine::{
        arg_types, chain,
        contract::FileContext,
        semantic_model::{SemanticModel, SolveOutcome},
        testkit,
    };
    use crate::type_checker::core::types::{Indirection, Lifetime, Type};
    use crate::types::{CallArg, EdgeKind};
    let source = "pub struct Doc; pub struct Holder;
        pub fn keep<'a>(p:&'a Doc)->&'a Doc { p }
        impl Holder { pub fn method<'a>(&self,p:&'a Doc)->&'a Doc { p } pub fn assoc<'a>(p:&'a Doc)->&'a Doc { p } }
        pub mod api { pub fn keep<'a>(p:&'a super::Doc)->&'a super::Doc { p } }
        pub fn run(receiver:Holder,p:Doc) { keep(&p); receiver.method(&p); Holder::assoc(&p); api::keep(&p); }";
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='source_args'\nversion='0.0.0'\nedition='2021'\n[lib]\npath='lib.rs'",
    )
    .unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, source).unwrap();
    let arena = Arc::new(TypeArena::new());
    let mut parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "lib.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    for reference in &mut parsed.refs {
        reference.call_args = vec![CallArg::Ident("unrelated".into())];
        if let Some(chain) = &mut reference.chain {
            for segment in &mut chain.segments {
                segment.call_args.clear();
            }
        }
    }
    let files = [parsed];
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let context = crate::indexer::project_context::build_project_context(dir.path());
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    let lookup = FileLookup::for_file(&tree, &files[0], &ids);
    let file = FileContext {
        file_path: "lib.rs".into(),
        language: "rust".into(),
        imports: vec![],
        file_namespace: None,
    };
    let profile = &crate::languages::rust_lang::profile::RUST_PROFILE;
    let model = SemanticModel::production();
    let mut seen = 0;
    for reference in files[0].refs.iter().filter(|r| r.kind == EdgeKind::Calls) {
        let selector = reference
            .chain
            .as_ref()
            .and_then(|c| c.segments.last())
            .map(|s| s.byte_offset)
            .unwrap_or(reference.byte_offset);
        lookup.set_cursor(reference.byte_offset);
        let args = lookup.source_call_arguments(selector).unwrap().unwrap();
        let actual = arg_types::resolve_arg_types(&lookup, &arena, args);
        assert_eq!(actual.len(), 1);
        let Type::Indirect {
            kind: Indirection::Reference(Lifetime::Inference { owner, byte }),
            ..
        } = arena.get(actual[0])
        else {
            panic!("source borrow required");
        };
        assert_eq!(
            owner,
            ids.row_id("lib.rs", reference.source_symbol_index).unwrap()
        );
        let CallArg::BorrowAt { span, .. } = &args[0] else {
            panic!("source operand required");
        };
        assert_eq!(byte, span.start);
        let mut context = testkit::ref_ctx(
            reference,
            &files[0].symbols[reference.source_symbol_index],
            vec![],
        );
        context.source_symbol_id = Some(owner);
        let SolveOutcome::Resolved(info) = model.get_symbol_info(&context, &file, &lookup, profile)
        else {
            panic!("call must bind");
        };
        let yielded = if reference
            .chain
            .as_ref()
            .is_none_or(|c| c.segments.len() < 2)
        {
            let callee = lookup.symbol_by_id(info.target_symbol_id).unwrap();
            chain::apply_call_args(
                &lookup,
                &arena,
                callee,
                selector,
                &reference.call_args,
                &[],
                arena.intern(Type::Unknown),
                None,
                info.resolved_yield_type
                    .or_else(|| lookup.return_type_id_of(callee.id)),
                profile.delegate_wrappers,
            )
        } else {
            info.resolved_yield_type
        };
        assert_eq!(
            yielded,
            Some(actual[0]),
            "{}: target correctness must not hide a lost region/type",
            reference.target_name
        );
        seen += 1;
    }
    assert_eq!(seen, 4);
}

#[test]
fn explicit_borrows_use_source_owner_regions_and_do_not_accept_guessed_operands_or_partial_spans() {
    use crate::indexer::symbol_ids::SymbolIds;
    use crate::type_checker::core::types::{Indirection, Lifetime, Mutability, Type};
    use crate::types::{CallArg, SourceSpan};
    let arena = Arc::new(TypeArena::new());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    let source = "fn first() {} fn second() {}";
    std::fs::write(&path, source).unwrap();
    let parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "lib.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let mut ids = SymbolIds::default();
    ids.set_rows("lib.rs".into(), vec![71, 72]);
    let files = [parsed];
    let tree = Compilation::build(&files, &ids, Arc::clone(&arena));
    let mut one = FileLookup::new(&tree, "rust");
    let mut two = FileLookup::new(&tree, "rust");
    let first = SourceSpan { start: 40, end: 42 };
    let second = SourceSpan { start: 50, end: 55 };
    one.borrow_sites.extend([
        (first, (71, Mutability::Shared)),
        (second, (71, Mutability::Mutable)),
    ]);
    two.borrow_sites.insert(first, (72, Mutability::Shared));
    let inner = arena.decl("ignored-display", 999);
    let shared = one.borrow_argument(first, inner).unwrap();
    assert_eq!(
        arena.get(shared),
        Type::Indirect {
            kind: Indirection::Reference(Lifetime::Inference {
                owner: 71,
                byte: 40
            }),
            mutability: Mutability::Shared,
            inner
        }
    );
    let mutable = one.borrow_argument(second, shared).unwrap();
    assert_eq!(
        arena.get(mutable),
        Type::Indirect {
            kind: Indirection::Reference(Lifetime::Inference {
                owner: 71,
                byte: 50
            }),
            mutability: Mutability::Mutable,
            inner: shared
        }
    );
    one.set_cursor(999);
    assert_eq!(one.borrow_argument(first, inner), Some(shared));
    assert_ne!(two.borrow_argument(first, inner), Some(shared));
    assert_eq!(one.method_call_region(first.start), None);
    assert_eq!(
        one.borrow_argument(SourceSpan { end: 43, ..first }, inner),
        None
    );
    assert_eq!(
        one.borrow_argument(first, arena.intern(Type::Unknown)),
        None
    );
    assert_eq!(
        one.borrow_argument(first, arena.class("ignored-display")),
        None
    );
    assert_eq!(
        super::super::super::arg_types::resolve_arg_types(
            &one,
            &arena,
            &[CallArg::BorrowAt {
                span: first,
                expr: Box::new(CallArg::IdentAt(SourceSpan { start: 41, end: 42 }))
            }]
        ),
        [arena.intern(Type::Unknown)]
    );
    one.borrow_sites.insert(first, (123456, Mutability::Shared));
    assert_eq!(
        one.borrow_argument(first, inner),
        None,
        "deleted owners cannot create regions"
    );
}

#[test]
fn selected_qualified_outputs_reuse_the_correct_explicit_argument_region_ids() {
    use crate::type_checker::core::types::{Indirection, Lifetime, Type};
    use crate::{indexer::write::write_parsed_files_with_origin, Database};
    let source = "pub struct Input; pub struct Doc; pub trait Choose {
        fn keep<'a,'b>(&'a self,x:&'b Doc)->&'a Self { self }
        fn other<'a,'b>(&'a self,x:&'b Doc)->&'b Doc { x }
        } impl Choose for Input {}
        pub fn run(p:Input,x:Doc) { <Input as Choose>::keep(&p,&x); <Input as Choose>::other(&p,&x); }";
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='borrow_test'\nversion='0.0.0'\nedition='2021'\n[lib]\npath='lib.rs'",
    )
    .unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, source).unwrap();
    let arena = Arc::new(TypeArena::new());
    let parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "lib.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let files = [parsed];
    let db = Database::open_in_memory().unwrap();
    let (_, ids) = write_parsed_files_with_origin(&db, &files, "internal", Some(&arena)).unwrap();
    let context = crate::indexer::project_context::build_project_context(dir.path());
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    let caller_slot = files[0]
        .symbols
        .iter()
        .position(|s| s.name == "run")
        .unwrap();
    let owner = ids.row_id("lib.rs", caller_slot).unwrap();
    let lookup = FileLookup::for_file(&tree, &files[0], &ids);
    let mut seen = 0;
    for segment in files[0]
        .refs
        .iter()
        .filter_map(|r| r.chain.as_ref())
        .filter_map(|c| c.segments.get(1))
    {
        if !lookup.qualified_call_site(segment.byte_offset) {
            continue;
        }
        lookup.set_cursor(segment.byte_offset);
        let actual =
            super::super::super::arg_types::resolve_arg_types(&lookup, &arena, &segment.call_args);
        assert_eq!(actual.len(), 2);
        for (&ty, arg) in actual.iter().zip(&segment.call_args) {
            let crate::types::CallArg::BorrowAt { span, .. } = arg else {
                panic!("borrow provenance");
            };
            let Type::Indirect {
                kind: Indirection::Reference(region),
                ..
            } = arena.get(ty)
            else {
                panic!("reference type");
            };
            assert_eq!(
                region,
                Lifetime::Inference {
                    owner,
                    byte: span.start
                }
            );
        }
        assert_ne!(actual[0], actual[1]);
        let selected = lookup
            .qualified_call(segment.byte_offset, &actual, &[])
            .unwrap()
            .ok()
            .unwrap();
        assert_eq!(
            selected.return_type,
            Some(
                actual[usize::from(
                    segment.byte_offset == source.find("::other(").unwrap() as u32 + 2
                )]
            )
        );
        seen += 1;
    }
    assert_eq!(seen, 2);
}

#[test]
fn method_region_identity_is_reused_by_site_and_never_by_spelling_or_cursor() {
    use crate::indexer::symbol_ids::SymbolIds;
    use crate::type_checker::core::types::{Lifetime, Type};
    let arena = Arc::new(TypeArena::new());
    let dir = tempfile::tempdir().unwrap();
    let mut ids = SymbolIds::default();
    let parsed: Vec<_> = [("first.rs", 71), ("second.rs", 72)]
        .into_iter()
        .map(|(path, id)| {
            let absolute_path = dir.path().join(path);
            std::fs::write(&absolute_path, "fn same() {}").unwrap();
            let file = crate::indexer::parse_file::parse_file_with_arena(
                &crate::walker::WalkedFile {
                    relative_path: path.into(),
                    absolute_path,
                    language: "rust",
                },
                &crate::languages::default_registry(),
                &arena,
            )
            .unwrap();
            assert_eq!(file.symbols.len(), 1);
            ids.set_rows(path.into(), vec![id]);
            file
        })
        .collect();
    let tree = Compilation::build(&parsed, &ids, Arc::clone(&arena));
    let mut one = FileLookup::new(&tree, "rust");
    let mut two = FileLookup::new(&tree, "rust");
    one.method_calls.extend([(41, 71), (42, 71), (43, 999)]);
    two.method_calls.insert(41, 72);
    let first = one.method_call_region(41).unwrap();
    one.set_cursor(900);
    assert_eq!(one.method_call_region(41), Some(first));
    assert_eq!(
        arena.get(first),
        Type::Region(Lifetime::Inference {
            owner: 71,
            byte: 41
        })
    );
    assert_ne!(one.method_call_region(42), Some(first));
    assert_ne!(two.method_call_region(41), Some(first));
    assert_eq!(one.method_call_region(40), None);
    assert_eq!(one.method_call_region(43), None);
}

#[test]
fn argument_reads_use_their_own_binding_and_position_without_moving_the_cursor() {
    use crate::indexer::resolve::engine::arg_types::resolve_arg_types;
    use crate::types::{CallArg, SourceSpan};
    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), Arc::clone(&arena));
    let mut graph = LexicalBindings::default();
    let root = graph.add_scope(None, 0, 100, true);
    let sibling = graph.add_scope(Some(root), 50, 90, true);
    let name = graph.intern("value");
    let outer = graph.declare(root, name, 1, None);
    let inner = graph.declare(sibling, name, 50, None);
    let first = SourceSpan { start: 20, end: 25 };
    let second = SourceSpan { start: 70, end: 75 };
    graph.argument_reads.insert(first, outer);
    graph.argument_reads.insert(second, inner);
    graph.writes.insert(1, outer);
    graph.writes.insert(2, inner);
    let mut lookup = FileLookup::new(&tree, "typescript");
    lookup.lexical = Some(super::super::super::lexical_cache::LexicalCache::new(
        &graph, &arena,
    ));
    let alpha = arena.decl("Same", 71);
    let beta = arena.decl("Same", 72);
    lookup.set_cursor(10);
    lookup.record_rhs_type(1, "ignored", alpha);
    lookup.set_cursor(60);
    lookup.record_rhs_type(2, "ignored", beta);
    assert_eq!(
        resolve_arg_types(
            &lookup,
            &arena,
            &[CallArg::IdentAt(first), CallArg::IdentAt(second)]
        ),
        [alpha, beta]
    );
    assert_eq!(
        lookup.local_type_id("value"),
        Some(beta),
        "argument reads must not move the active cursor"
    );
    let invalid = SourceSpan {
        start: first.start,
        end: first.end + 1,
    };
    assert_eq!(
        resolve_arg_types(&lookup, &arena, &[CallArg::IdentAt(invalid)]),
        [arena.intern(crate::type_checker::core::types::Type::Unknown)]
    );
}

#[test]
fn missing_lexical_row_blocks_the_ladder_and_target_spelling_is_not_identity() {
    use crate::indexer::resolve::engine::{
        semantic_model::{SemanticModel, SolveOutcome},
        testkit,
    };
    use crate::indexer::symbol_ids::SymbolIds;
    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), Arc::clone(&arena));
    let mut graph = LexicalBindings::default();
    let scope = graph.add_scope(None, 0, 100, true);
    let name = graph.intern("callback");
    let binding = graph.declare(scope, name, 1, None);
    graph.references.insert(25, binding);
    graph.attach_symbol(1, binding);
    let mut lookup = FileLookup::new(&tree, "typescript");
    lookup.lexical = Some(super::super::super::lexical_cache::LexicalCache::new(
        &graph, &arena,
    ));
    let mut reference = testkit::call_ref("not-the-binding-spelling");
    reference.byte_offset = 25;
    let source = testkit::source_symbol("caller");
    let context = testkit::ref_ctx(&reference, &source, vec![]);
    let file = testkit::file_ctx(vec![], None);
    let solver = SemanticModel::production();
    let profile = &crate::languages::typescript::profile::TYPESCRIPT_PROFILE;
    lookup.set_cursor(25);
    assert!(matches!(
        solver.get_symbol_info(&context, &file, &lookup, profile),
        SolveOutcome::Unresolved(None)
    ));
    let mut ids = SymbolIds::default();
    ids.set_rows("a.ts".into(), vec![10, 20]);
    lookup
        .lexical
        .as_mut()
        .unwrap()
        .install_declarations("a.ts", &ids);
    match solver.get_symbol_info(&context, &file, &lookup, profile) {
        SolveOutcome::Resolved(info) => {
            assert_eq!(info.target_symbol_id, 20);
            assert_eq!(info.strategy, "lexical_binding");
        }
        _ => panic!("binding should follow the stored occurrence ID, not target spelling"),
    }
}

#[test]
fn scoped_writes_require_identity_and_clear_removes_inference() {
    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), Arc::clone(&arena));
    let mut graph = LexicalBindings::default();
    let scope = graph.add_scope(None, 0, 100, true);
    let name = graph.intern("value");
    let binding = graph.declare(scope, name, 1, None);
    graph.writes.insert(7, binding);
    graph.initializers.insert(7);
    graph.symbols.insert(3, binding);
    let parameter = crate::types::SourceSpan { start: 1, end: 6 };
    graph.declarations.insert(parameter, binding);
    let mut lookup = FileLookup::new(&tree, "typescript");
    lookup.lexical = Some(super::super::super::lexical_cache::LexicalCache::new(
        &graph, &arena,
    ));
    let ty = arena.class("Alpha");
    lookup.set_cursor(10);

    lookup.record_local_type_id("value".into(), ty);
    lookup.record_local_type("value".into(), "Beta".into());
    lookup.record_local_callable_head("value".into(), "Beta.save".into());
    assert!(lookup.has_local_binding("value"));
    assert_eq!(lookup.local_type_id("value"), None);
    assert_eq!(lookup.local_callable_id("value"), None);

    lookup.record_contextual_type(crate::types::SourceSpan { start: 1, end: 5 }, ty);
    assert_eq!(
        lookup.local_type_id("value"),
        None,
        "A partial span is not declaration identity"
    );
    lookup.record_contextual_type(parameter, ty);
    assert_eq!(lookup.local_type_id("value"), Some(ty));
    lookup.clear_local_cache();
    lookup.set_cursor(10);

    // The text adapter argument cannot redirect a numeric reference binding.
    lookup.record_rhs_type(7, "unrelated", ty);
    lookup.record_rhs_cause(
        7,
        "unrelated",
        Cause::new(Some(42), CauseKind::UncapturedReturn),
    );
    lookup.record_symbol_callable(3, "unrelated", 42);
    assert_eq!(
        lookup.local_type_id("value"),
        Some(arena.intern(crate::type_checker::core::types::Type::Unknown)),
        "a failed RHS must not leave the previous inferred type as evidence"
    );
    assert_eq!(lookup.local_type_id("unrelated"), None);
    assert_eq!(lookup.local_callable_id("value"), Some(42));
    assert_eq!(lookup.root_cause_hint("value").unwrap().symbol_id, Some(42));

    lookup.clear_local_cache();
    lookup.set_cursor(10);
    assert!(lookup.has_local_binding("value"));
    assert_eq!(lookup.local_type_id("value"), None);
    assert_eq!(lookup.local_callable_id("value"), None);
    assert!(lookup.root_cause_hint("value").is_none());
}

#[test]
fn callback_only_bindings_use_spans_and_block_legacy_same_name_fallbacks() {
    use crate::types::SourceSpan;

    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), Arc::clone(&arena));
    let mut graph = LexicalBindings::default();
    let root = graph.add_scope(None, 0, 100, true);
    let outer_scope = graph.add_scope(Some(root), 10, 90, true);
    let inner_scope = graph.add_scope(Some(outer_scope), 30, 70, true);
    let name = graph.intern("item");
    let outer = graph.declare(outer_scope, name, 10, None);
    let inner = graph.declare(inner_scope, name, 30, None);
    let outer_parameter = SourceSpan { start: 10, end: 14 };
    let inner_parameter = SourceSpan { start: 30, end: 34 };
    graph.declarations.insert(outer_parameter, outer);
    graph.declarations.insert(inner_parameter, inner);
    graph.references.insert(20, outer);
    graph.references.insert(40, inner);

    let mut lookup = FileLookup::new(&tree, "rust");
    lookup.callback_lexical = Some(super::super::super::lexical_cache::LexicalCache::new(
        &graph, &arena,
    ));
    let outer_type = arena.class("Outer");
    let inner_type = arena.class("Inner");
    let conflict = arena.class("Conflict");
    let legacy = arena.class("Legacy");

    // A legacy name write before either callback remains usable outside their
    // scopes, but must not become evidence for either `item` declaration.
    lookup.set_cursor(5);
    lookup.record_local_type_id("item".into(), legacy);
    assert_eq!(lookup.local_type_id("item"), Some(legacy));
    lookup.set_cursor(25);
    assert!(
        !lookup.has_local_binding("item"),
        "a scope-visible callback name is insufficient without an exact read"
    );
    assert_eq!(
        lookup.local_type_id("item"),
        Some(legacy),
        "an unmodeled inner shadow remains eligible for legacy flow evidence"
    );
    lookup.set_cursor(20);
    assert!(lookup.has_local_binding("item"));
    assert_eq!(
        lookup.local_type_id("item"),
        None,
        "a mapped callback with no context must still block the legacy name"
    );
    assert_eq!(lookup.local_type("item"), None);

    // Exact declaration spans select their own binding. A nearby span is not
    // identity and cannot seed either callback.
    lookup.record_contextual_type(
        SourceSpan {
            start: outer_parameter.start,
            end: outer_parameter.end - 1,
        },
        conflict,
    );
    lookup.record_contextual_type(outer_parameter, outer_type);
    lookup.record_contextual_type(inner_parameter, inner_type);

    lookup.set_cursor(20);
    assert!(lookup.has_local_binding("item"));
    assert_eq!(lookup.local_type_id("item"), Some(outer_type));
    assert_eq!(lookup.local_type("item"), None);
    assert_eq!(
        lookup
            .local_reference(20)
            .and_then(|value| value.value_type),
        Some(outer_type),
        "the outer read must use its stored binding identity"
    );

    lookup.set_cursor(40);
    assert!(lookup.has_local_binding("item"));
    assert_eq!(lookup.local_type_id("item"), Some(inner_type));
    assert_eq!(
        lookup
            .local_reference(40)
            .and_then(|value| value.value_type),
        Some(inner_type),
        "the nested same-name callback must not read outer or legacy evidence"
    );

    // Two incompatible contexts for one exact parameter degrade only that
    // binding to Unknown; the outer callback remains concrete.
    lookup.record_contextual_type(inner_parameter, conflict);
    assert_eq!(
        lookup.local_type_id("item"),
        Some(arena.intern(crate::type_checker::core::types::Type::Unknown))
    );
    lookup.set_cursor(20);
    assert_eq!(lookup.local_type_id("item"), Some(outer_type));

    lookup.clear_local_cache();
    lookup.set_cursor(20);
    assert_eq!(lookup.local_type_id("item"), None);
    lookup.set_cursor(5);
    assert_eq!(lookup.local_type_id("item"), None);
}

#[test]
fn full_lexical_context_precedes_an_overlapping_callback_graph() {
    use crate::types::SourceSpan;

    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), Arc::clone(&arena));
    let parameter = SourceSpan { start: 10, end: 14 };
    let mut full = LexicalBindings::default();
    let full_scope = full.add_scope(None, 0, 100, true);
    let full_name = full.intern("item");
    let full_binding = full.declare(full_scope, full_name, 10, None);
    full.declarations.insert(parameter, full_binding);
    full.references.insert(20, full_binding);

    let mut callback = LexicalBindings::default();
    let callback_scope = callback.add_scope(None, 0, 100, true);
    let callback_name = callback.intern("item");
    let callback_binding = callback.declare(callback_scope, callback_name, 10, None);
    callback.declarations.insert(parameter, callback_binding);
    callback.references.insert(20, callback_binding);

    let mut lookup = FileLookup::new(&tree, "rust");
    lookup.lexical = Some(super::super::super::lexical_cache::LexicalCache::new(
        &full, &arena,
    ));
    lookup.callback_lexical = Some(super::super::super::lexical_cache::LexicalCache::new(
        &callback, &arena,
    ));
    let full_type = arena.class("Full");
    lookup.record_contextual_type(parameter, full_type);
    lookup.set_cursor(20);

    assert_eq!(lookup.local_type_id("item"), Some(full_type));
    assert_eq!(
        lookup
            .callback_lexical
            .as_ref()
            .and_then(|cache| cache.attested_local_type("item"))
            .flatten(),
        None,
        "the callback graph must not receive a context owned by full lexical data"
    );
}

#[test]
fn scala_lambda_at_span_seeds_its_exact_callback_root() {
    use crate::types::CallArg;
    use rustc_hash::FxHashMap;

    let source = "class Item { def touch(): Unit = () }\nclass Runner { def use(f: Item => Unit): Unit = () }\nobject P { def run(r: Runner) = r.use(x => x.touch()) }\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("callbacks.scala");
    std::fs::write(&path, source).unwrap();
    let arena = Arc::new(TypeArena::new());
    let parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "callbacks.scala".into(),
            absolute_path: path,
            language: "scala",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let files = [parsed];
    let callback = files[0]
        .refs
        .iter()
        .find(|reference| {
            reference.target_name == "use"
                && reference
                    .call_args
                    .iter()
                    .any(|arg| matches!(arg, CallArg::LambdaAt { .. }))
        })
        .expect("Scala use call must retain its LambdaAt argument");
    let parameter = callback
        .call_args
        .iter()
        .find_map(|arg| match arg {
            CallArg::LambdaAt { params } => params.first().copied().flatten(),
            _ => None,
        })
        .expect("Scala callback parameter must retain an exact source span");
    let root = files[0]
        .refs
        .iter()
        .find(|reference| {
            reference.target_name == "touch"
                && reference
                    .chain
                    .as_ref()
                    .and_then(|chain| chain.segments.first())
                    .is_some_and(|segment| segment.name == "x")
        })
        .expect("x.touch must retain x as its chain root")
        .byte_offset;
    let graph = files[0]
        .flow
        .callback_lexical
        .as_ref()
        .expect("Scala LambdaAt must build callback-only identity");
    assert!(graph.declarations.contains_key(&parameter));
    assert!(graph.references.contains_key(&root));

    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(&files, &ids, Arc::clone(&arena));
    let lookup = FileLookup::for_file(&tree, &files[0], &ids);
    lookup.set_cursor(root);
    assert_eq!(lookup.local_type_id("x"), None);

    let callee = crate::indexer::resolve::engine::testkit::sym_with_sig(
        900,
        "use",
        "Runner.use",
        "method",
        "callbacks.scala",
        "use(f: (Item) => Unit): Unit",
    );
    crate::indexer::resolve::engine::lambda_seed::seed_lambda_params(
        &lookup,
        &arena,
        &callee,
        &callback.call_args,
        arena.class("Runner"),
        None,
        &FxHashMap::default(),
        &[],
    );

    lookup.set_cursor(root);
    assert_eq!(lookup.local_type_id("x"), Some(arena.class("Item")));
    assert_eq!(
        lookup
            .local_reference(root)
            .and_then(|value| value.value_type),
        Some(arena.class("Item")),
        "the contextual span must be readable only through the exact callback root"
    );
}

fn assert_explicit_lambda_at_span_seeds_its_exact_callback_root(
    language: &'static str,
    extension: &str,
    source: &str,
    signature: &str,
) {
    use crate::types::CallArg;
    use rustc_hash::FxHashMap;

    let dir = tempfile::tempdir().unwrap();
    let relative_path = format!("callbacks.{extension}");
    let path = dir.path().join(&relative_path);
    std::fs::write(&path, source).unwrap();
    let arena = Arc::new(TypeArena::new());
    let parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: relative_path.clone(),
            absolute_path: path,
            language,
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let files = [parsed];
    let callback = files[0]
        .refs
        .iter()
        .find(|reference| {
            reference.target_name == "use"
                && reference
                    .call_args
                    .iter()
                    .any(|arg| matches!(arg, CallArg::LambdaAt { .. }))
        })
        .unwrap_or_else(|| {
            panic!(
                "{language} use call must retain its LambdaAt argument: {:#?}",
                files[0].refs
            )
        });
    let parameter = callback
        .call_args
        .iter()
        .find_map(|arg| match arg {
            CallArg::LambdaAt { params } => params.first().copied().flatten(),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{language} callback parameter needs an exact source span"));
    assert_eq!(
        &source[parameter.start as usize..parameter.end as usize],
        "item"
    );
    let root = files[0]
        .refs
        .iter()
        .find(|reference| {
            reference.target_name == "touch"
                && reference
                    .chain
                    .as_ref()
                    .and_then(|chain| chain.segments.first())
                    .is_some_and(|segment| segment.name == "item")
        })
        .unwrap_or_else(|| panic!("{language} item.touch must retain item as its chain root"))
        .byte_offset;
    let graph = files[0]
        .flow
        .callback_lexical
        .as_ref()
        .unwrap_or_else(|| panic!("{language} LambdaAt must build callback-only identity"));
    assert!(graph.declarations.contains_key(&parameter));
    assert!(graph.references.contains_key(&root));

    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(&files, &ids, Arc::clone(&arena));
    let lookup = FileLookup::for_file(&tree, &files[0], &ids);
    lookup.set_cursor(root);
    assert_eq!(lookup.local_type_id("item"), None);

    let callee = crate::indexer::resolve::engine::testkit::sym_with_sig(
        901,
        "use",
        "Runner.use",
        "method",
        &relative_path,
        signature,
    );
    crate::indexer::resolve::engine::lambda_seed::seed_lambda_params(
        &lookup,
        &arena,
        &callee,
        &callback.call_args,
        arena.class("Runner"),
        None,
        &FxHashMap::default(),
        &[],
    );

    lookup.set_cursor(root);
    assert_eq!(lookup.local_type_id("item"), Some(arena.class("Item")));
    assert_eq!(
        lookup
            .local_reference(root)
            .and_then(|value| value.value_type),
        Some(arena.class("Item")),
        "the contextual span must flow through the exact {language} callback reference"
    );
}

#[test]
fn kotlin_lambda_at_span_seeds_its_exact_callback_root() {
    assert_explicit_lambda_at_span_seeds_its_exact_callback_root(
        "kotlin",
        "kt",
        "fun run(r: Runner) { r.use { item -> item.touch() } }\n",
        "use(f: (Item) -> Unit): Unit",
    );
}

#[test]
fn swift_lambda_at_span_seeds_its_exact_callback_root() {
    assert_explicit_lambda_at_span_seeds_its_exact_callback_root(
        "swift",
        "swift",
        "class Item { func touch() {} }\nclass Runner { func use(_ f: (Item) -> Void) {} }\nfunc run(_ r: Runner) { r.use { item in item.touch() } }\n",
        "use(f: (Item) -> Void): Void",
    );
}
