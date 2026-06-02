// =============================================================================
// indexer/local_refs_tests.rs — operator-token ref filtering.
// =============================================================================

use super::*;
use crate::types::{EdgeKind, ExtractedRef};

fn call_ref(target: &str) -> ExtractedRef {
    ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

#[test]
fn drops_bare_and_paren_operators() {
    let mut refs = vec![
        call_ref("+"),
        call_ref("=="),
        call_ref("<>"),
        call_ref("(=)"),
        call_ref("(+)"),
        call_ref("::"),
        call_ref("|>"),
        call_ref("."),
    ];
    filter_operator_refs(&mut refs);
    assert!(
        refs.is_empty(),
        "all operator tokens dropped, left: {:?}",
        refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

#[test]
fn keeps_real_identifiers_and_mixed_tokens() {
    let mut refs = vec![
        call_ref("foo"),     // identifier
        call_ref("Foo"),     // type-ish
        call_ref("(Fn(K))"), // mis-parsed expr — has letters, not an operator
        call_ref("a+b"),     // has letters
        call_ref("map"),
    ];
    let before = refs.len();
    filter_operator_refs(&mut refs);
    assert_eq!(refs.len(), before, "no real identifiers dropped");
}

#[test]
fn keeps_operator_refs_that_carry_module() {
    // A module-bearing ref is an import, never a bare operator primitive.
    let mut with_module = call_ref("+");
    with_module.module = Some("ops".to_string());
    let mut refs = vec![with_module];
    filter_operator_refs(&mut refs);
    assert_eq!(refs.len(), 1, "module-bearing ref kept");
}
