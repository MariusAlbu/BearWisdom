// =============================================================================
// prolog/coverage_tests.rs
//
// Node-kind coverage for PrologPlugin::symbol_node_kinds() and ref_node_kinds().
// Grammar returns None; extraction is performed by the clause-aware line scanner.
//
// symbol_node_kinds: predicate_definition, module_declaration, use_module
// ref_node_kinds:    use_module, goal
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_fact_produces_function_symbol() {
    // A Prolog fact: `head.` → predicate_definition → Function
    // Names are in functor/arity format: animal/1
    let r = extract::extract("animal(dog).\nanimal(cat).\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name.starts_with("animal")),
        "Prolog fact should produce Function(animal/N); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_rule_produces_function_symbol() {
    // A Prolog rule: `head :- body.` → predicate_definition → Function
    // Name is functor/arity: foo/1
    let r = extract::extract("foo(X) :- bar(X).\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name.starts_with("foo")),
        "Prolog rule should produce Function(foo/N); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_rule_body_produces_calls() {
    // Goals in a rule body → Calls edges
    let r = extract::extract("parent(X, Y) :- mother(X, Y).\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "mother"),
        "rule body goal should produce Calls(mother); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_use_module_produces_imports() {
    // `:- use_module(library(lists)).` → Imports ref
    let r = extract::extract(":- use_module(library(lists)).\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "use_module should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional symbol_node_kinds — module_declaration → Namespace
// ---------------------------------------------------------------------------

#[test]
fn cov_module_declaration_produces_namespace() {
    // `:- module(mymod, [pred/1]).` → Namespace symbol
    let r = extract::extract(":- module(mymod, [pred/1]).\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Namespace && s.name == "mymod"),
        "module declaration should produce Namespace(mymod); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Zero-arity predicate (atom, no parens) → fact with arity 0
#[test]
fn cov_nullary_fact_produces_function_symbol() {
    let r = extract::extract("connected.\n");
    assert!(
        r.symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Function && s.name.starts_with("connected")),
        "nullary fact should produce Function(connected/0); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional ref_node_kinds — ensure_loaded → Imports
// ---------------------------------------------------------------------------

#[test]
fn cov_ensure_loaded_library_produces_imports() {
    // `:- ensure_loaded(library(lists)).` → Imports ref
    let r = extract::extract(":- ensure_loaded(library(lists)).\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "ensure_loaded(library(...)) should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_ensure_loaded_path_produces_imports() {
    // `:- ensure_loaded('utils/helpers').` → Imports ref to path
    let r = extract::extract(":- ensure_loaded('utils/helpers').\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "ensure_loaded(path) should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

/// use_module with path (not library) → Imports ref to path string
#[test]
fn cov_use_module_path_produces_imports() {
    let r = extract::extract(":- use_module('lib/utils').\n");
    assert!(
        r.refs
            .iter()
            .any(|rf| rf.kind == EdgeKind::Imports && rf.target_name.contains("utils")),
        "use_module(path) should produce Imports ref to path; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

/// Rule with multi-goal body — each non-builtin goal → Calls
#[test]
fn cov_multi_goal_body_produces_multiple_calls() {
    let r = extract::extract("grandparent(X, Z) :- parent(X, Y), parent(Y, Z).\n");
    let calls: Vec<_> = r.refs.iter().filter(|rf| rf.kind == EdgeKind::Calls).collect();
    assert!(
        calls.len() >= 2,
        "rule with two body goals should produce at least 2 Calls refs; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

/// Regression: list-literal unification goals (`[list] = _182`,
/// `[_147] = _184`) must not be mis-extracted as Calls refs. These
/// shapes appear thousands of times across SWI-Prolog's tests/xsb/
/// XSB-compatibility tests and used to emit phantom unresolved
/// targets like `[atom] = _95`.
#[test]
fn cov_list_unification_does_not_emit_call() {
    let src = "gencut__1(_174,_176,_178,_180) :- [list] = _182, [_147] = _184, normalize_result([_182, _184], [_174, _176]).\n";
    let r = extract::extract(src);
    let targets: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::Calls)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        targets.iter().all(|t| !t.starts_with('[')),
        "list-literal LHS must not be emitted as a Calls target; got: {:?}",
        targets
    );
    // The genuine call (normalize_result) should still come through.
    assert!(
        targets.iter().any(|t| *t == "normalize_result"),
        "expected normalize_result Calls ref; got: {:?}",
        targets
    );
}

/// Regression: variable / arithmetic / comparison-operator goals never
/// become Calls refs (their LHS is a variable or operator expression,
/// not a callable predicate name).
#[test]
fn cov_operator_goals_skipped() {
    let src = "p(X, Y, Z) :- X is Y + Z, Y > 0, _Tmp = X.\n";
    let r = extract::extract(src);
    let calls: Vec<_> = r.refs.iter().filter(|rf| rf.kind == EdgeKind::Calls).collect();
    assert!(
        calls.is_empty(),
        "operator-style goals should produce zero Calls refs; got: {:?}",
        calls.iter().map(|c| &c.target_name).collect::<Vec<_>>()
    );
}
