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

// ---------------------------------------------------------------------------
// Additional coverage for symbol variants from the rules
// ---------------------------------------------------------------------------

/// Level-77 data item → Variable symbol (level 77 = independent item)
#[test]
fn symbol_data_description_level_77() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. TEST.\n       DATA DIVISION.\n       WORKING-STORAGE SECTION.\n       77 WS-FLAG PIC X VALUE 'N'.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "WS-FLAG" && s.kind == SymbolKind::Variable),
        "expected Variable WS-FLAG from level 77 data description; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Subordinate data item (level 05) → Field symbol
#[test]
fn symbol_data_description_subordinate_field() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. TEST.\n       DATA DIVISION.\n       WORKING-STORAGE SECTION.\n       01 WS-RECORD.\n           05 WS-NAME PIC X(30).";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "WS-NAME" && s.kind == SymbolKind::Field),
        "expected Field WS-NAME from level 05 subordinate data item; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// CALL statement → Imports ref (CALL emits both Calls and Imports per extract.rs)
#[test]
fn ref_call_statement_also_imports() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. TEST.\n       PROCEDURE DIVISION.\n       MAIN-PARA.\n           CALL 'SUBPROG'.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "SUBPROG" && rf.kind == EdgeKind::Imports),
        "expected Imports SUBPROG from CALL statement; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// COPY in DATA DIVISION → Imports ref
#[test]
fn ref_copy_statement_in_data_division() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. TEST.\n       DATA DIVISION.\n       WORKING-STORAGE SECTION.\n           COPY DATALIB.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "DATALIB" && rf.kind == EdgeKind::Imports),
        "expected Imports DATALIB from COPY in DATA DIVISION; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `program_definition` → Namespace
/// PROGRAM-ID. Name in IDENTIFICATION DIVISION emits a Namespace symbol.
#[test]
fn symbol_program_definition() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. MYPROGRAM.\n       PROCEDURE DIVISION.\n       MAIN-PARA.\n           STOP RUN.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "MYPROGRAM" && s.kind == SymbolKind::Namespace),
        "expected Namespace MYPROGRAM from PROGRAM-ID; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: GO TO → Calls ref
#[test]
fn ref_goto_statement() {
    let src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. TEST.\n       PROCEDURE DIVISION.\n       MAIN-PARA.\n           GO TO END-PARA.\n       END-PARA.\n           STOP RUN.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "END-PARA" && rf.kind == EdgeKind::Calls),
        "expected Calls END-PARA from GO TO; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}
