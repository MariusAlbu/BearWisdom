// Tests for `correlate_member_symbol` — which declared member a
// `receiver.name = …` assignment targets.

use super::correlate_member_symbol;
use crate::types::{ExtractedSymbol, SymbolKind};

fn sym(
    name: &str,
    kind: SymbolKind,
    start: u32,
    end: u32,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind,
        visibility: None,
        start_line: start,
        end_line: end,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

/// One class spanning lines 0..9, its `factory` property on line 1, and a
/// `setUp` method spanning 3..6 that assigns it on line 4.
fn one_owner() -> Vec<ExtractedSymbol> {
    vec![
        sym("Host", SymbolKind::Class, 0, 9, None),
        sym("factory", SymbolKind::Property, 1, 1, Some(0)),
        sym("setUp", SymbolKind::Method, 3, 6, Some(0)),
    ]
}

#[test]
fn a_member_target_correlates_to_its_owners_property() {
    let symbols = one_owner();
    assert_eq!(correlate_member_symbol("factory", 4, &symbols), Some(1));
}

#[test]
fn a_member_target_never_correlates_across_owners() {
    let symbols = vec![
        sym("First", SymbolKind::Class, 0, 5, None),
        sym("factory", SymbolKind::Property, 1, 1, Some(0)),
        sym("Second", SymbolKind::Class, 7, 14, None),
        sym("factory", SymbolKind::Property, 8, 8, Some(2)),
        sym("setUp", SymbolKind::Method, 10, 13, Some(2)),
    ];
    assert_eq!(correlate_member_symbol("factory", 11, &symbols), Some(3));
}

#[test]
fn a_member_target_with_no_declared_property_declines() {
    let symbols = vec![
        sym("Host", SymbolKind::Class, 0, 9, None),
        sym("setUp", SymbolKind::Method, 3, 6, Some(0)),
    ];
    let before = symbols.len();
    assert_eq!(correlate_member_symbol("factory", 4, &symbols), None);
    assert_eq!(symbols.len(), before);
}

#[test]
fn a_same_named_local_variable_is_not_a_member_target() {
    let mut symbols = one_owner();
    symbols[1].kind = SymbolKind::Variable;
    assert_eq!(correlate_member_symbol("factory", 4, &symbols), None);
}
