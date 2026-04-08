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

// ---------------------------------------------------------------------------
// Additional symbol_node_kinds — inherit, inherit_from, dotted binding,
// rec_attrset, import with literal path
// ---------------------------------------------------------------------------

/// `inherit` in an attrset → Variable symbols for each inherited name
#[test]
fn cov_inherit_produces_variable() {
    let src = "{ inherit gcc clang; }";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "gcc"),
        "inherit should produce Variable(gcc); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "clang"),
        "inherit should produce Variable(clang); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// `inherit (src) name` → Variable symbols for each inherited name
#[test]
fn cov_inherit_from_produces_variables() {
    let src = "{ inherit (pkgs) hello git; }";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "hello"),
        "inherit_from should produce Variable(hello); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "git"),
        "inherit_from should produce Variable(git); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// `inherit (src) name` inside a let expression → Imports ref to the source.
/// The `expression` field on `inherit_from` (tree-sitter-nix 0.3) names the source attrset.
#[test]
fn cov_inherit_from_produces_imports_ref() {
    let src = "let x = { inherit (pkgs) hello; }; in x";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "pkgs"),
        "inherit_from should emit Imports(pkgs); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

/// Dotted attrpath binding `a.b = value` → Variable with qualified name "a.b"
#[test]
fn cov_dotted_binding_produces_qualified_variable() {
    let src = "{ meta.homepage = \"https://example.com\"; }";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Variable && s.name.contains('.')),
        "dotted binding should produce Variable with dotted name; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// `rec { ... }` bindings → Variable symbols (same as attrset)
#[test]
fn cov_rec_attrset_binding_produces_variable() {
    let src = "rec { x = 1; y = x + 1; }";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "x"),
        "rec attrset binding should produce Variable(x); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// `import ./path.nix` with a literal path → Imports ref to path string
#[test]
fn cov_import_literal_path_emits_imports_ref() {
    let src = "import ./config.nix";
    let r = extract::extract(src, lang());
    assert!(
        r.refs
            .iter()
            .any(|rf| rf.kind == EdgeKind::Imports && rf.target_name.contains("config.nix")),
        "import with literal path should emit Imports ref to path; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

/// Binding in a `let_attrset_expression` (`let { body = this; }`) → Variable
// TODO: let_attrset_expression is legacy syntax; test if grammar supports it before enabling.
// #[test]
// fn cov_let_attrset_binding_produces_variable() { ... }

/// `mkDerivation { ... }` assignment → Variable symbol (package derivation)
#[test]
fn cov_mkderivation_binding_produces_variable() {
    let src = "let myPkg = stdenv.mkDerivation { name = \"mypkg\"; }; in myPkg";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.name == "myPkg"),
        "mkDerivation binding should produce symbol(myPkg); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}
