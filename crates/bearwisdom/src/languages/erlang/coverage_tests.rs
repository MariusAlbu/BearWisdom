// =============================================================================
// erlang/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Symbol node kinds
// ---------------------------------------------------------------------------

/// symbol_node_kind: `fun_decl`  →  Function (name/arity)
#[test]
fn symbol_fun_decl() {
    let src = "-module(mymod).\n-export([foo/1]).\nfoo(X) -> bar(X).";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "foo/1" && s.kind == SymbolKind::Function),
        "expected Function foo/1; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `module_attribute`  →  Namespace
#[test]
fn symbol_module_attribute() {
    let src = "-module(mymod).\nfoo() -> ok.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "mymod" && s.kind == SymbolKind::Namespace),
        "expected Namespace mymod; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `record_decl`  →  Struct
#[test]
fn symbol_record_decl() {
    let src = "-module(user).\n-record(person, {name, age}).";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "person" && s.kind == SymbolKind::Struct),
        "expected Struct person; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `behaviour_attribute`  →  Implements edge (ref), no symbol emitted
/// The extractor records this as an Implements ref; verify at least one ref is produced.
#[test]
fn symbol_behaviour_attribute() {
    let src = "-module(myserver).\n-behaviour(gen_server).\n";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Implements && rf.target_name == "gen_server"),
        "expected Implements gen_server from behaviour_attribute; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Ref node kinds
// ---------------------------------------------------------------------------

/// ref_node_kind: `call`  →  Calls edge (local call)
#[test]
fn ref_call_local() {
    let src = "-module(mymod).\n-export([foo/1]).\nfoo(X) -> bar(X).";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "bar" && rf.kind == EdgeKind::Calls),
        "expected Calls bar; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `import_attribute`  →  Imports edge
#[test]
fn ref_import_attribute() {
    let src = "-module(mymod).\n-import(lists, [map/2, filter/2]).\nfoo() -> ok.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "lists"),
        "expected Imports lists from import_attribute; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `pp_include`  →  Imports edge
#[test]
fn ref_pp_include() {
    let src = "-module(mymod).\n-include(\"records.hrl\").\nfoo() -> ok.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name.contains("records")),
        "expected Imports records.hrl from pp_include; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `pp_include_lib`  →  Imports edge
#[test]
fn ref_pp_include_lib() {
    let src = "-module(mymod).\n-include_lib(\"stdlib/include/lists.hrl\").\nfoo() -> ok.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports from pp_include_lib; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `behaviour_attribute`  →  Implements edge (also listed as ref_node_kind)
#[test]
fn ref_behaviour_attribute() {
    let src = "-module(worker).\n-behaviour(gen_server).";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Implements),
        "expected Implements ref from behaviour_attribute; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}
