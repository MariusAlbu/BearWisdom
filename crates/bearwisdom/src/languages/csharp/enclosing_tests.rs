// =============================================================================
// csharp/enclosing_tests.rs — innermost-symbol attribution tests
// =============================================================================

use super::ScanAttribution;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};

fn sym(name: &str, kind: SymbolKind, start: u32, end: u32) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.into(),
        qualified_name: name.into(),
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
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn type_ref(name: &str, line: u32) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: name.into(),
        kind: EdgeKind::TypeRef,
        line,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

#[test]
fn innermost_symbol_wins_over_namespace_span() {
    // File-scoped namespace spans the whole file; the class and its method
    // nest inside. A ref on a method line must attribute to the METHOD.
    let symbols = vec![
        sym("Ns", SymbolKind::Namespace, 0, 100),
        sym("Ns.C", SymbolKind::Class, 10, 60),
        sym("Ns.C.M", SymbolKind::Method, 20, 30),
    ];
    let attr = ScanAttribution::build(&symbols, &[]);
    assert_eq!(attr.source_at(25), 2, "method line → method symbol");
    assert_eq!(attr.source_at(15), 1, "class line outside methods → class");
    assert_eq!(attr.source_at(5), 0, "namespace-only line → namespace");
    assert_eq!(attr.source_at(999), 0, "line past every span → fallback 0");
}

#[test]
fn claim_rejects_sites_already_emitted_by_symbol_passes() {
    let symbols = vec![sym("Ns", SymbolKind::Namespace, 0, 10)];
    let existing = vec![type_ref("User", 4)];
    let mut attr = ScanAttribution::build(&symbols, &existing);
    assert!(
        !attr.claim("User", 4),
        "same (name, line) as a symbol-pass ref must be skipped"
    );
    assert!(attr.claim("User", 5), "different line is a fresh site");
    assert!(!attr.claim("User", 5), "second scan emission at one site dedups");
}
