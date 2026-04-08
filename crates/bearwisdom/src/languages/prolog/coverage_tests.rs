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
