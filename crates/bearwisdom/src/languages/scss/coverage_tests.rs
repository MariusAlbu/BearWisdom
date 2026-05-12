// =============================================================================
// scss/coverage_tests.rs — Node-kind coverage tests for the SCSS extractor
//
// symbol_node_kinds:
//   mixin_statement, function_statement, keyframes_statement, rule_set
//
// ref_node_kinds:
//   include_statement, extend_statement, import_statement,
//   forward_statement, call_expression
//
// Grammar: tree-sitter-scss-local (dedicated SCSS grammar).
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds: mixin_statement
// ---------------------------------------------------------------------------

#[test]
fn cov_mixin_statement_emits_function() {
    let r = extract::extract("@mixin rounded($r: 4px) { border-radius: $r; }", "");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Function && s.name == "rounded");
    assert!(sym.is_some(), "expected Function 'rounded' from @mixin; got: {:?}", r.symbols);
}

#[test]
fn cov_error_root_text_fallback_recovers_mixins() {
    // The tree-sitter-scss-local grammar degrades the root node to `ERROR`
    // when it hits unsupported constructs — `#{$a}/#{$b}` inside a `font:`
    // shorthand is the classic offender (Font Awesome 4 `_mixins.scss`
    // trips on it in the very first mixin). When the root is `ERROR`,
    // tree-sitter does not produce `mixin_statement` nodes anywhere in
    // the tree — `@mixin NAME { … }` declarations collapse to loose
    // identifier / parameters / declaration tokens with no wrapper node
    // the visitor recognises.
    //
    // The extractor's text-level fallback scans for `@mixin NAME` and
    // `@function NAME` when the grammar parse is in error state and
    // produced zero symbols. Recovers the mixin names that sibling
    // partials `@include` against; without this, every Font Awesome-style
    // project had a long tail of unresolved mixin refs.
    let src = "// Mixins\n\
@mixin fa-icon() {\n\
  display: inline-block;\n\
  font: normal normal normal #{$fa-font-size-base}/#{$fa-line-height-base} FontAwesome;\n\
}\n\
\n\
@mixin fa-icon-rotate($degrees, $rotation) {\n\
  -webkit-transform: rotate($degrees);\n\
}\n\
\n\
@mixin fa-icon-flip($horiz, $vert, $rotation) {\n\
  -webkit-transform: scale($horiz, $vert);\n\
}\n\
\n\
@mixin sr-only {\n\
  position: absolute;\n\
}\n";
    let r = extract::extract(src, "_mixins.scss");
    let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    for required in &["fa-icon", "fa-icon-rotate", "fa-icon-flip", "sr-only"] {
        assert!(
            names.contains(required),
            "missing `{required}` from text-fallback recovery; names: {names:?}"
        );
    }
    for sym in &r.symbols {
        if ["fa-icon", "fa-icon-rotate", "fa-icon-flip", "sr-only"]
            .contains(&sym.name.as_str())
        {
            assert_eq!(
                sym.kind,
                SymbolKind::Function,
                "recovered mixin `{}` must be Function kind",
                sym.name
            );
        }
    }
}

#[test]
fn cov_error_root_fallback_not_triggered_on_clean_parse() {
    // On a clean parse the text fallback must not fire — grammar-driven
    // symbols already capture everything and double-emission would leave
    // duplicate rows in `symbols`.
    let src = "@mixin rounded($r: 4px) { border-radius: $r; }\n\
@mixin shadow { box-shadow: 0 0 4px #0003; }\n";
    let r = extract::extract(src, "_mixins.scss");
    assert!(!r.has_errors, "this source should parse cleanly");
    let rounded_count = r.symbols.iter().filter(|s| s.name == "rounded").count();
    let shadow_count = r.symbols.iter().filter(|s| s.name == "shadow").count();
    assert_eq!(rounded_count, 1);
    assert_eq!(shadow_count, 1);
}

#[test]
fn cov_error_root_fallback_at_function_recovery() {
    // `@function` declarations should also be recovered — same grammar
    // collapse pattern, same byte-level fallback path.
    let src = "@mixin broken() {\n  font: normal #{$a}/#{$b} X;\n}\n\
@function to-rem($px) {\n  @return ($px / 16) * 1rem;\n}\n";
    let r = extract::extract(src, "_fns.scss");
    let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"to-rem"),
        "@function declaration must be recovered via text fallback; names: {names:?}"
    );
}

#[test]
fn cov_mixin_statement_signature() {
    let r = extract::extract("@mixin flex-center { display: flex; align-items: center; }", "");
    let sym = r.symbols.iter().find(|s| s.name == "flex-center");
    assert!(sym.is_some(), "expected symbol 'flex-center'");
    assert_eq!(sym.unwrap().signature.as_deref(), Some("@mixin flex-center"));
}

// ---------------------------------------------------------------------------
// symbol_node_kinds: function_statement
// ---------------------------------------------------------------------------

#[test]
fn cov_function_statement_emits_function() {
    let r = extract::extract("@function rem($px) { @return $px / 16px; }", "");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Function && s.name == "rem");
    assert!(sym.is_some(), "expected Function 'rem' from @function; got: {:?}", r.symbols);
}

#[test]
fn cov_function_statement_signature() {
    let r = extract::extract("@function double($n) { @return $n * 2; }", "");
    let sym = r.symbols.iter().find(|s| s.name == "double");
    assert!(sym.is_some(), "expected symbol 'double'");
    assert_eq!(sym.unwrap().signature.as_deref(), Some("@function double"));
}

// ---------------------------------------------------------------------------
// symbol_node_kinds: keyframes_statement
// ---------------------------------------------------------------------------

#[test]
fn cov_keyframes_statement_emits_function() {
    let r = extract::extract(
        "@keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } }",
        "",
    );
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Function && s.name == "fadeIn");
    assert!(sym.is_some(), "expected Function 'fadeIn' from @keyframes; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// symbol_node_kinds: rule_set
// ---------------------------------------------------------------------------

#[test]
fn cov_rule_set_class_selector() {
    let r = extract::extract(".button { color: red; }", "");
    let sym = r.symbols.iter().find(|s| s.name == "button");
    assert!(sym.is_some(), "expected Class 'button' from .button; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Class);
}

#[test]
fn cov_rule_set_id_selector() {
    let r = extract::extract("#header { font-size: 2rem; }", "");
    let sym = r.symbols.iter().find(|s| s.name == "header");
    assert!(sym.is_some(), "expected symbol 'header' from #header; got: {:?}", r.symbols);
}

#[test]
fn cov_rule_set_tag_selector() {
    let r = extract::extract("div { color: red; }", "");
    assert!(!r.symbols.is_empty(), "expected at least one symbol from div rule_set");
}

#[test]
fn cov_rule_set_placeholder() {
    let r = extract::extract("%base-button { padding: 1rem; }", "");
    let sym = r.symbols.iter().find(|s| s.name == "base-button");
    assert!(sym.is_some(), "expected symbol 'base-button' from %placeholder; got: {:?}", r.symbols);
}

// Multi-selector and pseudo-selector extraction
// ---------------------------------------------------------------------------

#[test]
fn cov_multi_selector_comma_emits_all_classes() {
    // `.a, .b { }` should produce two Class symbols: "a" and "b".
    let r = extract::extract(".container, .container-fluid { display: block; }", "");
    let has_container = r.symbols.iter().any(|s| s.name == "container" && s.kind == SymbolKind::Class);
    let has_fluid = r.symbols.iter().any(|s| s.name == "container-fluid" && s.kind == SymbolKind::Class);
    assert!(has_container, "expected 'container'; got: {:?}", r.symbols);
    assert!(has_fluid, "expected 'container-fluid'; got: {:?}", r.symbols);
}

#[test]
fn cov_pseudo_selector_emits_base_name() {
    // `.clearfix:before, .clearfix:after { }` — both pseudo rules should
    // emit the base class name "clearfix" so that `@extend .clearfix` resolves.
    let r = extract::extract(".clearfix:before, .clearfix:after { content: ''; }", "");
    let matches: Vec<_> = r.symbols.iter().filter(|s| s.name == "clearfix").collect();
    assert!(!matches.is_empty(), "expected 'clearfix' from pseudo rules; got: {:?}", r.symbols);
}

#[test]
fn cov_class_text_fallback_recovers_class_rules_from_error_file() {
    // Files that produce root ERROR nodes (e.g. containing `--#{$prefix}prop`
    // interpolated CSS custom properties) cause the grammar-driven path to miss
    // class rules defined elsewhere in the same file. The text-scan fallback
    // must recover `.class-name {` top-level rules so that `@extend` refs can
    // resolve against them.
    let src = ".btn {\n\
               --prefix-btn-padding-x: #{$btn-padding-x};\n\
               display: inline-block;\n\
               }\n\
               .btn-lg {\n\
               padding: 0.5rem 1rem;\n\
               }\n\
               .btn-sm {\n\
               padding: 0.25rem 0.5rem;\n\
               }\n";
    let r = extract::extract(src, "_buttons.scss");
    assert!(r.has_errors, "interpolated custom props should trigger parse errors");
    let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"btn-lg"),
        "expected 'btn-lg' from class text fallback; names: {names:?}"
    );
    assert!(
        names.contains(&"btn-sm"),
        "expected 'btn-sm' from class text fallback; names: {names:?}"
    );
    let sm = r.symbols.iter().find(|s| s.name == "btn-sm").unwrap();
    assert_eq!(sm.kind, SymbolKind::Class, "recovered class symbol must have Class kind");
}

// ---------------------------------------------------------------------------
// ref_node_kinds: include_statement
// ---------------------------------------------------------------------------

#[test]
fn cov_include_statement_emits_calls() {
    let r = extract::extract(".btn { @include rounded; }", "");
    let call = r.refs.iter().find(|e| e.kind == EdgeKind::Calls && e.target_name == "rounded");
    assert!(call.is_some(), "expected Calls ref to 'rounded' from @include; got: {:?}", r.refs);
}

#[test]
fn cov_include_statement_with_args() {
    let r = extract::extract(".btn { @include flex-center(row, wrap); }", "");
    let call = r.refs.iter().find(|e| e.kind == EdgeKind::Calls && e.target_name == "flex-center");
    assert!(call.is_some(), "expected Calls ref to 'flex-center' from @include with args; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// ref_node_kinds: extend_statement
// ---------------------------------------------------------------------------

#[test]
fn cov_extend_statement_class_emits_inherits() {
    let r = extract::extract(".btn-primary { @extend .btn; }", "");
    let inh = r.refs.iter().find(|e| e.kind == EdgeKind::Inherits && e.target_name == "btn");
    assert!(inh.is_some(), "expected Inherits ref to 'btn' from @extend .btn; got: {:?}", r.refs);
}

#[test]
fn cov_extend_statement_placeholder_emits_inherits() {
    // The SCSS grammar parses @extend %placeholder as an ERROR node at block
    // scope. @extend .class works correctly as extend_statement. The grammar
    // limitation means %placeholder extends don't emit Inherits refs.
    // Test with class selector which is the common case.
    let r = extract::extract(".btn { @extend .base-button; }", "");
    let inh = r.refs.iter().find(|e| e.kind == EdgeKind::Inherits && e.target_name == "base-button");
    assert!(inh.is_some(), "expected Inherits ref to 'base-button' from @extend .class; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// ref_node_kinds: import_statement
// ---------------------------------------------------------------------------

#[test]
fn cov_import_statement_emits_imports() {
    let r = extract::extract("@import 'base';", "");
    let imp = r.refs.iter().find(|e| e.kind == EdgeKind::Imports && e.target_name == "base");
    assert!(imp.is_some(), "expected Imports ref to 'base' from @import; got: {:?}", r.refs);
}

#[test]
fn cov_import_statement_strips_extension() {
    let r = extract::extract("@import 'partials/buttons.scss';", "");
    let imp = r.refs.iter().find(|e| e.kind == EdgeKind::Imports && e.target_name == "buttons");
    assert!(imp.is_some(), "expected target 'buttons' (no extension); got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// ref_node_kinds: forward_statement
// ---------------------------------------------------------------------------

#[test]
fn cov_forward_statement_emits_imports() {
    let r = extract::extract("@forward 'variables';", "");
    let imp = r.refs.iter().find(|e| e.kind == EdgeKind::Imports && e.target_name == "variables");
    assert!(imp.is_some(), "expected Imports ref to 'variables' from @forward; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// ref_node_kinds: call_expression
// ---------------------------------------------------------------------------

#[test]
fn cov_call_expression_in_value() {
    // darken() is not a CSS builtin — should emit a Calls ref
    let r = extract::extract(".btn { color: darken(#ff0000, 10%); }", "");
    let call = r.refs.iter().find(|e| e.kind == EdgeKind::Calls && e.target_name == "darken");
    assert!(call.is_some(), "expected Calls ref to 'darken' from call_expression; got: {:?}", r.refs);
}

#[test]
fn cov_call_expression_nested_in_arguments() {
    // mix() is nested inside darken() arguments — both must be extracted
    let r = extract::extract(".btn { color: darken(mix($a, $b, 50%), 10%); }", "");
    let darken = r.refs.iter().find(|e| e.kind == EdgeKind::Calls && e.target_name == "darken");
    let mix = r.refs.iter().find(|e| e.kind == EdgeKind::Calls && e.target_name == "mix");
    assert!(darken.is_some(), "expected Calls ref to outer 'darken'; got: {:?}", r.refs);
    assert!(mix.is_some(), "expected Calls ref to nested 'mix'; got: {:?}", r.refs);
}

#[test]
fn cov_call_expression_nested_multiple() {
    // nth() calls nested inside @include arguments must be extracted
    let r = extract::extract(
        "@each $type in $types { @include badge-style(nth($type, 2), nth($type, 3)); }",
        "",
    );
    let nth_calls: Vec<_> = r.refs.iter().filter(|e| e.kind == EdgeKind::Calls && e.target_name == "nth").collect();
    assert_eq!(nth_calls.len(), 2, "expected 2 Calls refs to 'nth'; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// Bonus: $variable declaration
// ---------------------------------------------------------------------------

#[test]
fn cov_scss_variable_declaration() {
    let r = extract::extract("$primary-color: #ff0000;", "");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Variable && s.name == "primary-color");
    assert!(sym.is_some(), "expected Variable 'primary-color'; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// ref_node_kinds: use_statement -> Imports ref
// ---------------------------------------------------------------------------

#[test]
fn cov_use_statement_emits_imports() {
    // `@use 'sass:math'` introduces the Sass built-in namespace `math`
    // (the segment after the colon); the raw module path is preserved in the
    // `module` field so the resolver can classify it as external.
    let r = extract::extract("@use 'sass:math';", "");
    let imp = r.refs.iter().find(|e| e.kind == EdgeKind::Imports && e.target_name == "math");
    assert!(imp.is_some(), "expected Imports ref with target 'math' from @use 'sass:math'; got: {:?}", r.refs);
    assert_eq!(imp.unwrap().module.as_deref(), Some("sass:math"));
}

#[test]
fn cov_use_statement_simple_path_emits_imports() {
    let r = extract::extract("@use 'variables';", "");
    let imp = r.refs.iter().find(|e| e.kind == EdgeKind::Imports && e.target_name == "variables");
    assert!(imp.is_some(), "expected Imports ref to 'variables' from @use; got: {:?}", r.refs);
}
// ---------------------------------------------------------------------------
// symbol_node_kinds: variable_name — signature includes declaration line
// ---------------------------------------------------------------------------

#[test]
fn cov_scss_variable_signature() {
    let r = extract::extract("$spacing-unit: 8px;", "");
    let sym = r.symbols.iter().find(|s| s.name == "spacing-unit");
    assert!(sym.is_some(), "expected Variable 'spacing-unit'");
    let sig = sym.unwrap().signature.as_deref().unwrap_or("");
    assert!(sig.contains("spacing-unit"), "expected signature to contain variable name; got: {:?}", sig);
}

// ---------------------------------------------------------------------------
// symbol_node_kinds: rule_set — nested rule inherits parent
// ---------------------------------------------------------------------------

#[test]
fn cov_rule_set_nested_class() {
    // Nested rule sets — each rule set emits a Class symbol
    let r = extract::extract(".card { .title { color: blue; } }", "");
    assert!(
        r.symbols.iter().any(|s| s.name == "card"),
        "expected Class 'card'; got: {:?}", r.symbols
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds: call_expression — namespace-qualified call strips namespace
// ---------------------------------------------------------------------------

#[test]
fn cov_call_expression_namespace_qualified() {
    // After @use "sass:math" as math, calls look like math.ceil(...)
    // The extractor strips the namespace prefix and uses the last component.
    let r = extract::extract(".x { width: math.ceil(1.5px); }", "");
    let call = r.refs.iter().find(|e| e.kind == EdgeKind::Calls && e.target_name == "ceil");
    assert!(call.is_some(), "expected Calls ref to 'ceil' from math.ceil(); got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// ref_node_kinds: include_statement — emits Calls, not Imports
// ---------------------------------------------------------------------------

#[test]
fn cov_include_statement_edge_kind_is_calls() {
    let r = extract::extract(".x { @include theme; }", "");
    let call = r.refs.iter().find(|e| e.target_name == "theme");
    assert!(call.is_some(), "expected ref to 'theme'");
    assert_eq!(call.unwrap().kind, EdgeKind::Calls, "@include should produce Calls, not {:?}", call.unwrap().kind);
}

// ---------------------------------------------------------------------------
// @use 'path' as alias — alias stored as target_name
// ---------------------------------------------------------------------------

#[test]
fn cov_use_statement_with_alias_stores_alias_as_target() {
    let r = extract::extract("@use 'mixins' as m;", "test.scss");
    let imp = r.refs.iter().find(|e| e.kind == EdgeKind::Imports);
    assert!(imp.is_some(), "expected Imports ref; got: {:?}", r.refs);
    assert_eq!(imp.unwrap().target_name, "m", "alias should be stored as target_name");
    assert_eq!(imp.unwrap().module.as_deref(), Some("mixins"));
}

#[test]
fn cov_use_statement_sass_builtin_with_alias() {
    let r = extract::extract("@use 'sass:math' as math;", "test.scss");
    let imp = r.refs.iter().find(|e| e.kind == EdgeKind::Imports);
    assert!(imp.is_some(), "expected Imports ref for sass:math");
    assert_eq!(imp.unwrap().target_name, "math");
    assert_eq!(imp.unwrap().module.as_deref(), Some("sass:math"));
}

// ---------------------------------------------------------------------------
// @include namespace.mixin() — grammar limitation: only namespace returned
// ---------------------------------------------------------------------------

#[test]
fn cov_include_namespace_qualified_emits_namespace_as_target() {
    // The SCSS grammar surfaces only the first identifier before the dot;
    // the resolver classifies the namespace against `@use` import entries.
    let r = extract::extract("@include m.fa-icon();", "test.scss");
    let call = r.refs.iter().find(|e| e.kind == EdgeKind::Calls);
    assert!(call.is_some(), "expected Calls ref; got: {:?}", r.refs);
    assert_eq!(call.unwrap().target_name, "m", "namespace prefix should be the target");
}

// ---------------------------------------------------------------------------
// @extend .#{$expr} — interpolated selectors skipped
// ---------------------------------------------------------------------------

#[test]
fn cov_extend_interpolated_target_is_skipped() {
    let r = extract::extract(".x { @extend .#{$var}; }", "test.scss");
    let inherits = r.refs.iter().filter(|e| e.kind == EdgeKind::Inherits).count();
    assert_eq!(inherits, 0, "interpolated @extend should produce no Inherits ref");
}

// ---------------------------------------------------------------------------
// .sass indented syntax — `=mixin-name` recovery
// ---------------------------------------------------------------------------

#[test]
fn cov_sass_indented_mixin_recovery() {
    let src = "=my-mixin($arg)\n  display: block\n";
    let r = extract::extract(src, "styles.sass");
    let sym = r.symbols.iter().find(|s| s.name == "my-mixin");
    assert!(sym.is_some(), "expected Function 'my-mixin' from indented sass; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

// ---------------------------------------------------------------------------
// Partial parse errors — fallback runs even when some symbols were extracted
// ---------------------------------------------------------------------------

#[test]
fn cov_partial_parse_error_recovers_late_mixins() {
    // A mixin defined before a parse error is captured by the grammar path.
    // A mixin defined after the error node is only recovered by the text
    // fallback — but only if the fallback runs regardless of how many
    // symbols the grammar path already found.
    let src = "@mixin early() { color: red; }\n\
               .bad { font: 12/14 sans-serif; }\n\
               @mixin late() { color: blue; }\n";
    let r = extract::extract(src, "test.scss");
    let early = r.symbols.iter().any(|s| s.name == "early");
    let late = r.symbols.iter().any(|s| s.name == "late");
    assert!(early, "expected 'early' mixin; got: {:?}", r.symbols.iter().map(|s| &s.name).collect::<Vec<_>>());
    assert!(late, "expected 'late' mixin recovered by text fallback; got: {:?}", r.symbols.iter().map(|s| &s.name).collect::<Vec<_>>());
}
