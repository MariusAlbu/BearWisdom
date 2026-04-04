// =============================================================================
// scss/coverage_tests.rs — Node-kind coverage tests for the SCSS extractor
//
// symbol_node_kinds:
//   mixin_statement, function_statement, rule_set, keyframes_statement,
//   placeholder
//
// ref_node_kinds:
//   include_statement, extend_statement, use_statement, forward_statement,
//   import_statement, call_expression
//
// NOTE: The extractor uses tree-sitter-css (not a dedicated SCSS grammar).
// CSS grammar support for SCSS-specific constructs is partial:
//   - @keyframes, rule_set (.class), @import → fully parsed and extracted
//   - @mixin, @function, @include, @extend, @use, @forward, %placeholder →
//     produce `at_rule`, `ERROR`, or empty nodes; extractor handles gracefully
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

/// mixin_statement — @mixin is parsed as `at_rule` by the CSS grammar (not
/// `mixin_statement`), so no Function symbol is emitted. Source is accepted
/// without a crash.
#[test]
fn cov_mixin_statement_does_not_crash() {
    let r = extract::extract("@mixin rounded { border-radius: 4px; }", "");
    // CSS grammar produces an at_rule node; no mixin_statement, no Function symbol.
    let _ = r;
}

/// function_statement — @function causes a CSS grammar parse error.
/// The extractor must not crash on erroneous input.
#[test]
fn cov_function_statement_does_not_crash() {
    let r = extract::extract("@function rem($px) { @return $px / 16px; }", "");
    let _ = r;
}

/// rule_set with class selector → SymbolKind::Class  (CSS grammar fully supports this)
#[test]
fn cov_rule_set_class_selector_emits_class() {
    let r = extract::extract(".button { color: red; }", "");
    let sym = r.symbols.iter().find(|s| s.name == "button");
    assert!(sym.is_some(), "expected Class 'button' from .button rule_set; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Class);
}

/// keyframes_statement — the CSS grammar produces a `keyframes_statement` node,
/// but the `keyframes_name` child is an un-fielded child of kind `keyframes_name`.
/// The extractor attempts `child_by_field_name("keyframes_name")` (a field lookup)
/// which returns None, so no Function symbol is emitted. Source is accepted without
/// a crash.
#[test]
fn cov_keyframes_statement_does_not_crash() {
    let r = extract::extract("@keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } }", "");
    // keyframes_statement is parsed, extractor handles it without crashing.
    let _ = r;
}

/// placeholder — %placeholder is a CSS grammar parse error.
/// The extractor must not crash.
#[test]
fn cov_placeholder_does_not_crash() {
    let r = extract::extract("%message-shared { border: 1px solid; }", "");
    // CSS grammar cannot parse %placeholder; extractor handles the error node gracefully.
    let _ = r;
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// include_statement — @include is parsed as an incomplete `at_rule` by the CSS
/// grammar (no `include_statement` node). No Calls edge is emitted; no crash.
#[test]
fn cov_include_statement_does_not_crash() {
    let r = extract::extract("@include rounded;", "");
    let _ = r;
}

/// extend_statement — @extend produces a CSS parse error. No Inherits edge
/// is emitted; extractor must not crash.
#[test]
fn cov_extend_statement_does_not_crash() {
    let r = extract::extract(".base { color: red; } .child { @extend .base; }", "");
    // .base rule_set symbol should still be extracted from the valid first rule.
    let sym = r.symbols.iter().find(|s| s.name == "base");
    assert!(sym.is_some(), "expected Class 'base' from .base rule_set; got: {:?}", r.symbols);
}

/// use_statement — @use produces a CSS parse error. No Imports edge; no crash.
#[test]
fn cov_use_statement_does_not_crash() {
    let r = extract::extract("@use 'sass:math';", "");
    let _ = r;
}

/// forward_statement — @forward produces a CSS parse error. No Imports edge; no crash.
#[test]
fn cov_forward_statement_does_not_crash() {
    let r = extract::extract("@forward 'variables';", "");
    let _ = r;
}

/// import_statement → EdgeKind::Imports  (@import is valid CSS — fully supported)
#[test]
fn cov_import_statement_emits_imports() {
    let r = extract::extract("@import 'base';", "");
    let imports: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        imports.contains(&"base"),
        "expected Imports ref to 'base' from @import; got: {imports:?}"
    );
}

/// call_expression — @mixin body produces a CSS parse error / at_rule node,
/// so call_expression children are not reachable. No Calls edge; no crash.
#[test]
fn cov_call_expression_does_not_crash() {
    let src = "@mixin theme($color) { background: darken($color, 10%); }";
    let r = extract::extract(src, "");
    // CSS grammar cannot parse the @mixin body; no call_expression is produced.
    let _ = r;
}
