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

// ---------------------------------------------------------------------------
// Perl 5 OOP: `use parent` / `use base` → Inherits
// ---------------------------------------------------------------------------

/// `use parent 'Base'` → EdgeKind::Imports (line scanner emits Imports;
/// tree-sitter-based Inherits is not available without the grammar)
///
/// The current line scanner cannot distinguish `use parent` from a generic
/// module import, so it emits `Imports("parent")`.  We verify the ref is
/// present and mark the inheritance intent with a comment.
#[test]
fn ref_use_parent_emits_imports() {
    let r = extract("package Animal;\nuse parent 'Mammal';\n");
    // Line scanner emits Imports for any `use` that is not strict/warnings.
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports ref from use parent; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// `use base 'BaseClass'` → similar Imports ref
#[test]
fn ref_use_base_emits_imports() {
    let r = extract("package Dog;\nuse base 'Animal';\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports ref from use base; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// `sub new` — constructor heuristic
// ---------------------------------------------------------------------------

/// `sub new` → Function symbol (no special Constructor kind in line scanner)
///
/// The line scanner treats every `sub` uniformly as Function.  Constructor
/// detection is a higher-level concern layered on top.
#[test]
fn symbol_sub_new_is_function() {
    let r = extract("package Foo;\nsub new { my $class = shift; bless {}, $class; }\n");
    assert!(
        r.symbols.iter().any(|s| s.name == "new" && s.kind == SymbolKind::Function),
        "expected Function symbol 'new'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// `use constant` → treated as a module import by line scanner
// ---------------------------------------------------------------------------

/// `use constant NAME => VALUE` — `constant` pragma is parsed as a `use`
/// statement.  The line scanner emits Imports("constant").
#[test]
fn ref_use_constant_emits_imports() {
    let r = extract("use constant PI => 3.14;\n");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "constant" && rf.kind == EdgeKind::Imports),
        "expected Imports('constant') from use constant; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Versioned `use` → skip (numeric module names are version specifiers)
// ---------------------------------------------------------------------------

/// `use 5.020;` → no Imports ref emitted (version specifier, not a module)
#[test]
fn ref_use_version_skipped() {
    let r = extract("use 5.020;\n");
    assert!(
        r.refs.is_empty(),
        "expected no refs for use version; got {:?}",
        r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Package namespace is module-qualified
// ---------------------------------------------------------------------------

/// `package Foo::Bar::Baz` → Namespace symbol with nested name preserved
#[test]
fn symbol_nested_package_name_preserved() {
    let r = extract("package Foo::Bar::Baz;\n");
    assert!(
        r.symbols.iter().any(|s| s.name == "Foo::Bar::Baz" && s.kind == SymbolKind::Namespace),
        "expected Namespace 'Foo::Bar::Baz'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}
