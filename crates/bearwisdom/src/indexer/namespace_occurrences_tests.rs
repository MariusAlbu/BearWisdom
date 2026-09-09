use super::*;

#[test]
fn borrows_attest_exact_spans_mutability_and_physical_function_owners() {
    use crate::type_checker::core::types::Mutability;
    let source = "fn f(p:C) { C::take(&p, &mut p, &&p); let c=|| C::take(&p); C::take(&raw const p); fn f(p:C) { C::take(&p); } }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let extracted = crate::languages::rust_lang::extract::extract(source);
    let data = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &extracted.symbols,
        &extracted.refs,
    )
    .unwrap();
    assert_eq!(
        data.borrow_sites.len(),
        5,
        "closure/raw operators cannot borrow the outer function's identity"
    );
    let inner = source.rfind("fn f").unwrap();
    for (&span, &(slot, mutable)) in &data.borrow_sites {
        let text = &source[span.start as usize..span.end as usize];
        assert_eq!(
            mutable,
            if text == "&mut p" {
                Mutability::Mutable
            } else {
                Mutability::Shared
            }
        );
        assert_eq!(
            extracted.symbols[slot].start_col as usize,
            if span.start as usize > inner {
                inner
            } else {
                0
            }
        );
    }
    let no_owners = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &[],
        &extracted.refs,
    )
    .unwrap();
    assert!(no_owners.borrow_sites.is_empty());
}

#[test]
fn method_regions_capture_dot_calls_and_physical_owners_not_ufcs_or_function_values() {
    let source = "fn f(p: C) { p.make().make(); p.make::<C>(); C::make(&p); (p.make)(); let _c = || p.make(); fn f(p: C) { p.make(); } }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let extracted = crate::languages::rust_lang::extract::extract(source);
    let data = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &extracted.symbols,
        &extracted.refs,
    )
    .unwrap();
    let mut sites: Vec<_> = data
        .method_calls
        .iter()
        .map(|(&byte, &slot)| (byte as usize, extracted.symbols[slot].start_col as usize))
        .collect();
    sites.sort_unstable();
    let inner = source.find("fn f(p: C) { p.make(); }").unwrap();
    let expected: Vec<_> = source
        .match_indices("make")
        .enumerate()
        .filter(|(i, _)| ![3, 4].contains(i))
        .map(|(_, (byte, _))| (byte, if byte > inner { inner } else { 0 }))
        .collect();
    assert_eq!(sites, expected);
    assert_eq!(sites.len(), 5);
    let filtered = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &[],
        &extracted.refs,
    )
    .unwrap();
    assert!(
        filtered.method_calls.is_empty(),
        "removed declaration slots cannot supply caller IDs"
    );
}

#[test]
fn repeated_selectors_use_cst_addresses_before_canonicalization() {
    let source = "fn f() { let value = api::Doc::new().same().same(); value.same(); }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut extracted = crate::languages::rust_lang::extract::extract(source);
    stamp(
        tree.root_node(),
        source.as_bytes(),
        &crate::languages::rust_lang::namespaces::FORMS,
        &mut extracted.refs,
    );
    let outer = extracted
        .refs
        .iter()
        .filter(|r| r.kind == crate::types::EdgeKind::Calls)
        .filter_map(|r| r.chain.as_ref())
        .max_by_key(|c| c.segments.len())
        .unwrap();
    assert_eq!(outer.segments.len(), 5);
    assert_eq!(
        outer.segments[3].byte_offset,
        source.find("same()").unwrap() as u32
    );
    assert_eq!(
        outer.segments[4].byte_offset,
        source.find("same().same").unwrap() as u32 + 7
    );
    assert!(outer.segments[2..].iter().all(|s| s.is_call));
}

#[test]
fn qualified_type_paths_are_one_root_with_exact_selectors_and_argument_spans() {
    let source = "fn f(p: &Input, q: Doc) { <api::Input<Doc> as api::Choose<Doc>>::choose::<Doc>(p,q).touch(); }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut extracted = crate::languages::rust_lang::extract::extract(source);
    stamp(
        tree.root_node(),
        source.as_bytes(),
        &crate::languages::rust_lang::namespaces::FORMS,
        &mut extracted.refs,
    );
    let outer = extracted
        .refs
        .iter()
        .filter(|r| r.kind == crate::types::EdgeKind::Calls)
        .filter_map(|r| r.chain.as_ref())
        .max_by_key(|c| c.segments.len())
        .unwrap();
    assert_eq!(
        outer.segments.len(),
        3,
        "paths inside a qualified type are not value-chain hops"
    );
    assert!(
        extracted
            .refs
            .iter()
            .filter(|r| r.kind == crate::types::EdgeKind::Calls)
            .all(|r| r.module.is_none()),
        "a bracketed Self/trait expression is not module-prefix evidence"
    );
    assert_eq!(
        outer.segments[1].byte_offset,
        source.find("choose::<").unwrap() as u32
    );
    assert_eq!(
        outer.segments[2].byte_offset,
        source.find("touch()").unwrap() as u32
    );
    assert_eq!(outer.segments[1].type_args, ["Doc"]);
    let p = source.find("(p,q)").unwrap() as u32 + 1;
    assert_eq!(
        outer.segments[1].call_args,
        [
            crate::types::CallArg::IdentAt(crate::types::SourceSpan {
                start: p,
                end: p + 1
            }),
            crate::types::CallArg::IdentAt(crate::types::SourceSpan {
                start: p + 2,
                end: p + 3
            })
        ]
    );
}
