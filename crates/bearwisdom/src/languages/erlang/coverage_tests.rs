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

// ---------------------------------------------------------------------------
// Additional symbol node kinds from rules
// ---------------------------------------------------------------------------

/// symbol_node_kind: `type_alias`  →  TypeAlias
#[test]
fn symbol_type_alias() {
    let src = "-module(m).\n-type mytype() :: integer() | atom().\n";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "mytype" && s.kind == SymbolKind::TypeAlias),
        "expected TypeAlias mytype; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `opaque`  →  TypeAlias (opaque variant)
#[test]
fn symbol_opaque() {
    let src = "-module(m).\n-opaque handle() :: {pid(), reference()}.\n";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "handle" && s.kind == SymbolKind::TypeAlias),
        "expected TypeAlias handle from opaque; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `callback`  →  Method
#[test]
fn symbol_callback() {
    let src = "-module(m).\n-callback init(Args :: term()) -> {ok, State :: term()}.\n";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "init" && s.kind == SymbolKind::Method),
        "expected Method init from callback; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `wild_attribute`  →  Variable (custom attribute metadata)
#[test]
fn symbol_wild_attribute() {
    let src = "-module(m).\n-custom_tag(some_value).\n";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "custom_tag" && s.kind == SymbolKind::Variable),
        "expected Variable custom_tag from wild_attribute; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional ref node kinds from rules (not yet handled by extractor)
// ---------------------------------------------------------------------------

/// ref_node_kind: `call` with `remote` expr  →  Calls edge (qualified call)
#[test]
fn ref_call_remote() {
    let src = "-module(mymod).\n-export([foo/0]).\nfoo() -> lists:map(fun(X) -> X end, []).";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "map" && rf.kind == EdgeKind::Calls),
        "expected Calls map from remote call lists:map/2; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `internal_fun`  →  Calls edge (`fun foo/2` reference)
#[test]
fn ref_internal_fun() {
    let src = "-module(m).\nfoo() -> fun bar/2.\n";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "bar" && rf.kind == EdgeKind::Calls),
        "expected Calls bar from internal_fun; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `external_fun`  →  Calls edge (`fun mod:foo/2` reference)
#[test]
fn ref_external_fun() {
    let src = "-module(m).\nfoo() -> fun lists:map/2.\n";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "map" && rf.kind == EdgeKind::Calls),
        "expected Calls map from external_fun; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `record_expr`  →  Instantiates edge (`#record_name{...}`)
#[test]
fn ref_record_expr() {
    let src = "-module(m).\n-record(person, {name, age}).\nfoo() -> #person{name = \"bob\", age = 42}.\n";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "person" && rf.kind == EdgeKind::Instantiates),
        "expected Instantiates person from record_expr; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// Private function: fun_decl not in export list → Visibility::Private
#[test]
fn symbol_fun_decl_private() {
    let src = "-module(mymod).\nbar(X) -> X.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "bar/1" && s.kind == SymbolKind::Function),
        "expected Function bar/1; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Zero-arity function — arity should be 0.
#[test]
fn symbol_fun_decl_zero_arity() {
    let src = "-module(mymod).\n-export([start/0]).\nstart() -> ok.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "start/0" && s.kind == SymbolKind::Function),
        "expected Function start/0; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}
