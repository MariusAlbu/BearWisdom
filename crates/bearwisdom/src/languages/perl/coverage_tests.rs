// =============================================================================
// perl/coverage_tests.rs — Coverage tests for the Perl line-scanner extractor
//
// symbol_node_kinds: []  (line scanner — no tree-sitter node kinds)
// ref_node_kinds:    []
//
// Both slices are empty because the extractor is tree-sitter-free.
// Tests exercise the three logical extraction paths: package, sub, use.
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Symbols produced by the line scanner
// ---------------------------------------------------------------------------

/// `package` declaration → Namespace symbol
#[test]
fn symbol_package_namespace() {
    let r = extract("package MyModule;\nuse strict;\n");
    assert!(
        r.symbols.iter().any(|s| s.name == "MyModule" && s.kind == SymbolKind::Namespace),
        "expected Namespace MyModule; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// `sub` declaration → Function symbol
#[test]
fn symbol_sub_function() {
    let r = extract("sub foo { my $x = shift; return $x; }\n");
    assert!(
        r.symbols.iter().any(|s| s.name == "foo" && s.kind == SymbolKind::Function),
        "expected Function foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Multiple `sub` declarations in one file
#[test]
fn symbol_multiple_subs() {
    let r = extract("sub alpha { 1 }\nsub beta { 2 }\n");
    let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"alpha") && names.contains(&"beta"),
        "expected both alpha and beta; got {:?}", names
    );
}

/// `package` + `sub` together
#[test]
fn symbol_package_and_sub() {
    let r = extract("package MyModule;\nuse strict;\nsub foo { my $x = shift; bar($x); }\n");
    let kinds: Vec<SymbolKind> = r.symbols.iter().map(|s| s.kind).collect();
    assert!(
        kinds.contains(&SymbolKind::Namespace),
        "expected Namespace; got {:?}", kinds
    );
    assert!(
        kinds.contains(&SymbolKind::Function),
        "expected Function; got {:?}", kinds
    );
}

// ---------------------------------------------------------------------------
// References produced by the line scanner
// ---------------------------------------------------------------------------

/// `use Module` → Imports ref
#[test]
fn ref_use_imports() {
    let r = extract("package MyModule;\nuse strict;\nuse POSIX qw(floor);\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports from use; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// `use strict` and `use warnings` are skipped (not emitted as refs)
#[test]
fn ref_use_strict_warnings_skipped() {
    let r = extract("use strict;\nuse warnings;\n");
    assert!(
        r.refs.is_empty(),
        "expected no refs for use strict/warnings; got {:?}",
        r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}

/// `use` with module name preserved exactly
#[test]
fn ref_use_module_name() {
    let r = extract("use Scalar::Util qw(blessed);\n");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Scalar::Util" && rf.kind == EdgeKind::Imports),
        "expected Imports Scalar::Util; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}
