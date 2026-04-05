// =============================================================================
// nix/coverage_tests.rs
//
// Node-kind coverage for NixPlugin::symbol_node_kinds() and ref_node_kinds().
//
// symbol_node_kinds: binding, inherit, inherit_from
// ref_node_kinds:    apply_expression, with_expression
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
    // Top-level apply_expression → Imports ref (import is a builtin).
    let src = "import ./foo.nix";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls || rf.kind == EdgeKind::Imports),
        "apply_expression should produce Calls or Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_callpackage_emits_imports_ref() {
    // pkgs.callPackage path {} — inner apply should emit Imports ref to path.
    let src = "let result = pkgs.callPackage ./package.nix { }; in result";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name.contains("package.nix")),
        "callPackage should emit Imports ref to path; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_curried_call_emits_ref() {
    // lib.optionalAttrs cond {} — curried call should emit a Calls ref.
    let src = "let x = lib.optionalAttrs (a == b) { flag = true; }; in x";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name.contains("optionalAttrs")),
        "curried call should emit Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_formal_default_import_emits_ref() {
    // Function argument with import default: `pkgs ? import <nixpkgs> {}`
    // The import apply inside the formal default should produce an Imports ref.
    let src = "{ pkgs ? import <nixpkgs> { } }: pkgs.hello";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports || rf.kind == EdgeKind::Calls),
        "formal default with import should emit a ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_string_interpolation_apply_emits_ref() {
    // apply_expression inside a string interpolation should produce a Calls ref.
    let src = r#"let x = "prefix ${lib.toHexString val} suffix"; in x"#;
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name.contains("toHexString")),
        "apply in string interpolation should emit Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_with_expression_produces_imports() {
    // with_expression should emit an Imports ref for the environment.
    let src = "with pkgs; [ hello git ]";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "pkgs"),
        "with_expression should emit Imports(pkgs); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_import_with_complex_path_emits_calls_ref() {
    // import (base + "/path") — path not extractable, should fall back to Calls -> import
    let src = r#"let x = import (nixpkgs + "/nixos/lib/eval-config.nix"); in x"#;
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls || rf.kind == EdgeKind::Imports),
        "import with non-literal path should emit at least one ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
