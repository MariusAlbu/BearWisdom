// =============================================================================
// vba/coverage_tests.rs
//
// Node-kind coverage for VbaPlugin::symbol_node_kinds() and ref_node_kinds().
// Grammar returns None; extraction is performed by the case-insensitive line scanner.
//
// symbol_node_kinds: sub_declaration, function_declaration, class_module,
//                   property_declaration, variable_declaration
// ref_node_kinds:    call_statement
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_sub_declaration_produces_function() {
    let r = extract::extract("Sub MySub()\n    MsgBox \"Hello\"\nEnd Sub\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "MySub"),
        "Sub should produce Function(MySub); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_function_declaration_produces_function() {
    let r = extract::extract("Function Square(x As Integer) As Integer\n    Square = x * x\nEnd Function\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "Square"),
        "Function should produce Function(Square); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_class_module_produces_class() {
    // VBA class module marker: `Attribute VB_Name = "ClassName"`
    let r = extract::extract("Attribute VB_Name = \"MyClass\"\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "MyClass"),
        "VB_Name attribute should produce Class(MyClass); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_call_statement_produces_calls() {
    // `Call SubName` inside a sub → Calls ref
    let r = extract::extract("Sub Main()\n    Call Helper\nEnd Sub\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "Helper"),
        "Call statement should produce Calls(Helper); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
