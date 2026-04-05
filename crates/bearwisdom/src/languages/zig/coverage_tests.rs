// =============================================================================
// zig/coverage_tests.rs
//
// Node-kind coverage for ZigPlugin::symbol_node_kinds() and ref_node_kinds().
// Grammar is tree-sitter-zig; extraction uses the line scanner.
//
// symbol_node_kinds: function_declaration, test_declaration
// ref_node_kinds:    call_expression, builtin_function
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_function_declaration_produces_function() {
    let r = extract::extract("fn add(a: i32, b: i32) i32 {\n    return a + b;\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "add"),
        "fn should produce Function(add); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_pub_function_produces_function() {
    let r = extract::extract("pub fn main() void {\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "main"),
        "pub fn should produce Function(main); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_struct_declaration_produces_struct() {
    let r = extract::extract("const Point = struct {\n    x: f32,\n    y: f32,\n};");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Struct && s.name == "Point"),
        "const struct should produce Struct(Point); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_test_declaration_produces_test() {
    let r = extract::extract("test \"addition\" {\n    try std.testing.expect(1 + 1 == 2);\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Test),
        "test block should produce Test symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Functions nested inside `return struct { ... }` (generic/comptime types) are extracted.
#[test]
fn cov_comptime_generic_fn_extracted() {
    let r = extract::extract(
        "pub fn Container(comptime T: type) type {\n\
         return struct {\n\
             pub fn init(v: T) T {\n\
                 return v;\n\
             }\n\
         };\n\
         }",
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "init"),
        "fn inside return struct should produce init; got: {:?}",
        r.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

/// Methods inside enum bodies are extracted.
#[test]
fn cov_enum_method_extracted() {
    let r = extract::extract(
        "pub const Dir = enum {\n\
             north, south,\n\
             pub fn opposite(self: Dir) Dir {\n\
                 return switch (self) { .north => .south, .south => .north };\n\
             }\n\
         };",
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "opposite"),
        "fn inside enum should produce opposite; got: {:?}",
        r.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// `@import` produces an Imports ref.
#[test]
fn cov_import_produces_imports_ref() {
    let r = extract::extract("const std = @import(\"std\");");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "@import should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

/// Builtin function calls inside function bodies produce Calls refs.
#[test]
fn cov_builtin_call_in_body_produces_ref() {
    let r = extract::extract("pub fn foo() void {\n    const x: i32 = @intCast(42);\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "@intCast"),
        "@intCast in body should produce a Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// Top-level `@This()` produces a Calls ref.
#[test]
fn cov_this_at_toplevel_produces_ref() {
    let r = extract::extract("const Self = @This();");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "@This"),
        "@This at top level should produce a Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// Aggregate coverage check against the zig-ly test project.
/// Asserts symbol ≥ 95% and ref ≥ 95%.
#[test]
#[ignore]
fn assert_coverage_above_95_pct() {
    use crate::query::coverage::analyze_coverage;
    use std::path::Path;

    let project = Path::new("F:/Work/Projects/TestProjects/zig-ly");
    if !project.exists() {
        eprintln!("SKIP: test project not found at {}", project.display());
        return;
    }

    let results = analyze_coverage(project);
    let cov = results.iter().find(|c| c.language == "zig").expect("zig coverage not found");

    eprintln!(
        "zig: sym={:.1}% ({}/{}) ref={:.1}% ({}/{}) files={}",
        cov.symbol_coverage.percent,
        cov.symbol_coverage.matched_nodes,
        cov.symbol_coverage.expected_nodes,
        cov.ref_coverage.percent,
        cov.ref_coverage.matched_nodes,
        cov.ref_coverage.expected_nodes,
        cov.file_count,
    );
    for k in &cov.symbol_kinds {
        eprintln!("  SYM {:>30}: {:.1}% ({}/{})", k.kind, k.percent, k.matched, k.occurrences);
    }
    for k in &cov.ref_kinds {
        eprintln!("  REF {:>30}: {:.1}% ({}/{})", k.kind, k.percent, k.matched, k.occurrences);
    }

    assert!(
        cov.symbol_coverage.percent >= 95.0,
        "symbol coverage {:.1}% < 95%",
        cov.symbol_coverage.percent
    );
    assert!(
        cov.ref_coverage.percent >= 95.0,
        "ref coverage {:.1}% < 95%",
        cov.ref_coverage.percent
    );
}
