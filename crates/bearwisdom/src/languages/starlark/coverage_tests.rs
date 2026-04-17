// =============================================================================
// starlark/coverage_tests.rs
//
// Node-kind coverage for StarlarkPlugin::symbol_node_kinds() and ref_node_kinds().
// Grammar is tree-sitter-starlark; extraction also uses the line scanner.
//
// symbol_node_kinds: function_definition, assignment
// ref_node_kinds:    call
// =============================================================================

use super::{predicates, extract};
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_function_definition_produces_function() {
    let r = extract::extract("def my_rule():\n    pass\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "my_rule"),
        "def should produce Function(my_rule); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_rule_assignment_produces_function() {
    // name = rule(...) → Function (rule definition)
    let r = extract::extract("my_binary = rule(\n    implementation = _impl,\n)\n");
    assert!(
        r.symbols.iter().any(|s| s.name == "my_binary"),
        "rule assignment should produce symbol(my_binary); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_plain_assignment_produces_variable() {
    // A simple constant assignment → Variable
    let r = extract::extract("VERSION = \"1.0.0\"\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "VERSION"),
        "assignment should produce Variable(VERSION); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_load_produces_imports() {
    let r = extract::extract("load(\"//tools:defs.bzl\", \"cc_binary\")\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "load() should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_function_call_produces_calls() {
    let r = extract::extract("def build():\n    native.cc_binary(name = \"app\")\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "function call should produce Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional symbol_node_kinds — provider, struct, aspect, repository_rule,
// decorated_definition, test rule assignment
// ---------------------------------------------------------------------------

/// `name = provider(...)` → Struct symbol
#[test]
fn cov_provider_assignment_produces_struct() {
    let r = extract::extract("MyInfo = provider(\n    fields = [\"value\"],\n)\n");
    assert!(
        r.symbols.iter().any(|s| s.name == "MyInfo" && s.kind == SymbolKind::Struct),
        "provider assignment should produce Struct(MyInfo); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// `name = struct(...)` → Struct symbol
#[test]
fn cov_struct_assignment_produces_struct() {
    let r = extract::extract("MY_STRUCT = struct(field_a = 1, field_b = 2)\n");
    assert!(
        r.symbols.iter().any(|s| s.name == "MY_STRUCT" && s.kind == SymbolKind::Struct),
        "struct assignment should produce Struct(MY_STRUCT); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// `name = aspect(...)` → Function symbol (rule-like)
#[test]
fn cov_aspect_assignment_produces_function() {
    let r = extract::extract("my_aspect = aspect(\n    implementation = _impl,\n    attr_aspects = [\"deps\"],\n)\n");
    assert!(
        r.symbols.iter().any(|s| s.name == "my_aspect" && s.kind == SymbolKind::Function),
        "aspect assignment should produce Function(my_aspect); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// `name = repository_rule(...)` → Function symbol (rule-like)
#[test]
fn cov_repository_rule_assignment_produces_function() {
    let r = extract::extract("my_repo_rule = repository_rule(\n    implementation = _impl,\n)\n");
    assert!(
        r.symbols.iter().any(|s| s.name == "my_repo_rule" && s.kind == SymbolKind::Function),
        "repository_rule assignment should produce Function(my_repo_rule); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// `name = some_test_rule(...)` where callee ends in `_test` → Test symbol
#[test]
fn cov_test_rule_assignment_produces_test() {
    let r = extract::extract("my_test = cc_test(\n    name = \"my_test\",\n)\n");
    assert!(
        r.symbols.iter().any(|s| s.name == "my_test" && s.kind == SymbolKind::Test),
        "cc_test assignment should produce Test(my_test); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional ref_node_kinds — load with alias, attribute call
// ---------------------------------------------------------------------------

/// `load(...)` with keyword alias argument → Imports ref for the aliased symbol
#[test]
fn cov_load_with_alias_produces_imports() {
    let r = extract::extract("load(\"//tools:defs.bzl\", my_cc = \"cc_binary\")\n");
    // Should produce at least the module-level Imports ref.
    let imports: Vec<_> = r.refs.iter().filter(|rf| rf.kind == EdgeKind::Imports).collect();
    assert!(
        !imports.is_empty(),
        "load() with alias arg should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

/// `load(...)` with multiple positional symbol names → one Imports ref per symbol
#[test]
fn cov_load_multiple_symbols_produces_multiple_imports() {
    let r = extract::extract("load(\"//lib:foo.bzl\", \"bar\", \"baz\")\n");
    let imports: Vec<_> = r.refs.iter().filter(|rf| rf.kind == EdgeKind::Imports).collect();
    // Expect at least 3: module label + "bar" + "baz"
    assert!(
        imports.len() >= 3,
        "load() with 2 symbol args should produce >= 3 Imports refs; got {} refs: {:?}",
        imports.len(),
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

/// `native.cc_library(...)` call — attribute-style callee → Calls ref
#[test]
fn cov_native_attribute_call_produces_calls() {
    let r = extract::extract("def build():\n    native.cc_library(name = \"lib\", srcs = [\"a.cc\"])\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "native.cc_library(...) should produce a Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Builtin classification — is_starlark_builtin
// ---------------------------------------------------------------------------

/// `native.*` prefix always matches regardless of the specific method name.
#[test]
fn builtin_native_prefix_matches_any_method() {
    assert!(predicates::is_starlark_builtin("native.cc_binary"));
    assert!(predicates::is_starlark_builtin("native.cc_library"));
    assert!(predicates::is_starlark_builtin("native.cc_test"));
    assert!(predicates::is_starlark_builtin("native.py_library"));
    assert!(predicates::is_starlark_builtin("native.java_test"));
    assert!(predicates::is_starlark_builtin("native.genrule"));
    assert!(predicates::is_starlark_builtin("native.some_future_rule"));
    assert!(predicates::is_starlark_builtin("native"));
}

/// Bazel built-in `select(...)` is recognised.
#[test]
fn builtin_select_is_external() {
    assert!(predicates::is_starlark_builtin("select"));
}

/// `Label(...)` constructor is recognised.
#[test]
fn builtin_label_is_external() {
    assert!(predicates::is_starlark_builtin("Label"));
}

/// Project-defined names are not falsely classified as builtins.
#[test]
fn builtin_user_function_is_not_external() {
    assert!(!predicates::is_starlark_builtin("my_custom_rule"));
    assert!(!predicates::is_starlark_builtin("_impl"));
    assert!(!predicates::is_starlark_builtin("build_target"));
}

// ---------------------------------------------------------------------------
// load() ref shapes — verify target_name and module are set correctly
// ---------------------------------------------------------------------------

/// load() from external repo emits module starting with '@'.
#[test]
fn load_external_repo_module_starts_with_at() {
    let r = extract::extract(
        "load(\"@bazel_skylib//lib:paths.bzl\", \"paths\")\n",
    );
    let import_refs: Vec<_> = r.refs.iter().filter(|rf| rf.kind == EdgeKind::Imports).collect();
    assert!(!import_refs.is_empty(), "should produce Imports refs");
    assert!(
        import_refs.iter().any(|rf| rf.module.as_deref().unwrap_or("").starts_with('@')),
        "external load() module should start with '@'; got: {:?}",
        import_refs.iter().map(|rf| &rf.module).collect::<Vec<_>>()
    );
}

/// load() from internal workspace emits module starting with '//'.
#[test]
fn load_internal_module_starts_with_double_slash() {
    let r = extract::extract(
        "load(\"//tools/build_defs:foo.bzl\", \"my_rule\")\n",
    );
    let import_refs: Vec<_> = r.refs.iter().filter(|rf| rf.kind == EdgeKind::Imports).collect();
    assert!(!import_refs.is_empty());
    assert!(
        import_refs.iter().any(|rf| rf.module.as_deref().unwrap_or("").starts_with("//")),
        "internal load() module should start with '//'; got: {:?}",
        import_refs.iter().map(|rf| &rf.module).collect::<Vec<_>>()
    );
}

/// String label arguments to rules (e.g. `deps = ["//path:target"]`) are NOT
/// extracted as code refs — they are data strings, not symbol references.
#[test]
fn string_label_deps_not_extracted_as_refs() {
    let r = extract::extract(
        "cc_library(\n    name = \"mylib\",\n    deps = [\"//other:lib\", \"@external//pkg:dep\"],\n)\n",
    );
    let call_refs: Vec<_> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::Calls)
        .collect();
    for rf in &call_refs {
        assert!(
            !rf.target_name.contains("//") && !rf.target_name.starts_with('@'),
            "string label '{}' should not be extracted as a code ref",
            rf.target_name
        );
    }
}

