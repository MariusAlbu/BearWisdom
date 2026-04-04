// =============================================================================
// cobol/coverage_tests.rs — One test per declared symbol_node_kind and ref_node_kind
//
// COBOL uses a line-oriented scanner, not tree-sitter.
// symbol_node_kinds: ["paragraph", "section", "data_description",
//                     "perform_statement", "call_statement", "copy_statement"]
// ref_node_kinds:    ["perform_statement", "call_statement", "copy_statement"]
//
// The kind names are logical names listed in mod.rs, not actual tree-sitter node
// kinds.  Each test exercises the corresponding scanner detection path.
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

/// paragraph name in PROCEDURE DIVISION → Function symbol
#[test]
fn symbol_paragraph() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. HELLO.\n       PROCEDURE DIVISION.\n       MAIN-PARA.\n           DISPLAY 'HELLO'.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "MAIN-PARA" && s.kind == SymbolKind::Function),
        "expected Function MAIN-PARA; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// section heading in PROCEDURE DIVISION → Function symbol
#[test]
fn symbol_section() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. TEST.\n       PROCEDURE DIVISION.\n       INIT-SECTION SECTION.\n       STEP-1.\n           STOP RUN.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "INIT-SECTION" && s.kind == SymbolKind::Function),
        "expected Function INIT-SECTION; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// data item (level 01) in DATA DIVISION → Variable symbol
#[test]
fn symbol_data_description() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. TEST.\n       DATA DIVISION.\n       WORKING-STORAGE SECTION.\n       01 WS-COUNTER PIC 9(4).";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "WS-COUNTER" && s.kind == SymbolKind::Variable),
        "expected Variable WS-COUNTER; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// PERFORM statement → Calls ref  [symbol_node_kinds entry; ref side tested below]
#[test]
fn symbol_perform_statement_produces_ref() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. TEST.\n       PROCEDURE DIVISION.\n       MAIN-PARA.\n           PERFORM WORK-PARA.\n       WORK-PARA.\n           STOP RUN.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "WORK-PARA" && rf.kind == EdgeKind::Calls),
        "expected Calls WORK-PARA from PERFORM; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// CALL statement → Calls + Imports refs  [symbol_node_kinds entry]
#[test]
fn symbol_call_statement_produces_ref() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. TEST.\n       PROCEDURE DIVISION.\n       MAIN-PARA.\n           CALL 'SUBPROG'.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "expected Calls ref from CALL; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// COPY statement → Imports ref  [symbol_node_kinds entry]
///
/// The line scanner handles COPY in PROCEDURE DIVISION and in the catch-all
/// non-Data/non-Procedure divisions.  Place COPY in PROCEDURE DIVISION where
/// it is always processed.
#[test]
fn symbol_copy_statement_produces_ref() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. TEST.\n       PROCEDURE DIVISION.\n       MAIN-PARA.\n           COPY COPYLIB.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports ref from COPY; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// perform_statement → Calls ref
#[test]
fn ref_perform_statement() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. TEST.\n       PROCEDURE DIVISION.\n       MAIN-PARA.\n           PERFORM WORK-PARA.\n       WORK-PARA.\n           STOP RUN.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "WORK-PARA" && rf.kind == EdgeKind::Calls),
        "expected Calls WORK-PARA; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// call_statement → Calls ref
#[test]
fn ref_call_statement() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. TEST.\n       PROCEDURE DIVISION.\n       MAIN-PARA.\n           CALL 'EXTPROG'.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "EXTPROG" && rf.kind == EdgeKind::Calls),
        "expected Calls EXTPROG; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// copy_statement → Imports ref
#[test]
fn ref_copy_statement() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. TEST.\n       PROCEDURE DIVISION.\n       MAIN-PARA.\n           COPY MYLIB.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "MYLIB" && rf.kind == EdgeKind::Imports),
        "expected Imports MYLIB; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}
