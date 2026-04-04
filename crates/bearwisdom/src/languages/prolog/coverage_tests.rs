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
