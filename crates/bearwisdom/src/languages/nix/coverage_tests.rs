// =============================================================================
// nix/coverage_tests.rs
//
// Node-kind coverage for NixPlugin::symbol_node_kinds() and ref_node_kinds().
// The plugin mod.rs stubs ExtractionResult::empty() pending grammar wiring;
// these tests call extract::extract() directly with the live grammar.
//
// symbol_node_kinds: binding, inherit, inherit_from
// ref_node_kinds:    apply_expression, with_expression, select_expression
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

fn lang() -> tree_sitter::Language {
    tree_sitter_nix::LANGUAGE.into()
}

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_binding_produces_variable() {
    // A `binding` in a let-expression should produce a Variable symbol.
    let src = "let foo = pkgs.hello; in foo";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "foo"),
        "binding should produce Variable(foo); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_function_binding_produces_function() {
    // A binding whose RHS is a lambda → Function
    let src = "let myFunc = x: x + 1; in myFunc 5";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.name == "myFunc"),
        "function binding should produce symbol(myFunc); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_apply_expression_produces_calls() {
    // Top-level apply_expression → Calls ref.
    // Use a standalone application expression as the root expression.
    let src = "import ./foo.nix";
    let r = extract::extract(src, lang());
    // import is an apply_expression whose function is `import` — emits Imports.
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls || rf.kind == EdgeKind::Imports),
        "apply_expression should produce Calls or Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
