// =============================================================================
// clojure/coverage_tests.rs — One test per declared symbol_node_kind and ref_node_kind
//
// symbol_node_kinds: ["list_lit"]
// ref_node_kinds:    ["list_lit", "sym_lit"]
//
// Clojure's grammar uses list_lit for all declarations and calls alike.
// Tests exercise the logical distinctions the extractor makes inside list_lit.
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds — list_lit (declaration forms)
// ---------------------------------------------------------------------------

/// list_lit matched as `ns` → Namespace symbol
#[test]
fn symbol_list_lit_ns() {
    let r = extract("(ns myapp.core (:require [clojure.string :as str]))");
    assert!(
        r.symbols.iter().any(|s| s.name == "myapp.core" && s.kind == SymbolKind::Namespace),
        "expected Namespace myapp.core; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// list_lit matched as `defn` → Function symbol
#[test]
fn symbol_list_lit_defn() {
    let r = extract("(defn foo [x] x)");
    assert!(
        r.symbols.iter().any(|s| s.name == "foo" && s.kind == SymbolKind::Function),
        "expected Function foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// list_lit matched as `defn-` (private) → Function symbol
#[test]
fn symbol_list_lit_defn_private() {
    let r = extract("(defn- hidden [x] x)");
    assert!(
        r.symbols.iter().any(|s| s.name == "hidden" && s.kind == SymbolKind::Function),
        "expected Function hidden; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// list_lit matched as `defmacro` → Function symbol (macro)
#[test]
fn symbol_list_lit_defmacro() {
    let r = extract("(defmacro my-macro [x] x)");
    assert!(
        r.symbols.iter().any(|s| s.name == "my-macro" && s.kind == SymbolKind::Function),
        "expected Function my-macro; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// list_lit matched as `def` → Variable symbol
#[test]
fn symbol_list_lit_def() {
    let r = extract("(def max-retries 3)");
    assert!(
        r.symbols.iter().any(|s| s.name == "max-retries" && s.kind == SymbolKind::Variable),
        "expected Variable max-retries; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// list_lit matched as `defrecord` → Struct symbol
#[test]
fn symbol_list_lit_defrecord() {
    let r = extract("(defrecord Point [x y])");
    assert!(
        r.symbols.iter().any(|s| s.name == "Point" && s.kind == SymbolKind::Struct),
        "expected Struct Point; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// list_lit matched as `defprotocol` → Interface symbol
#[test]
fn symbol_list_lit_defprotocol() {
    let r = extract("(defprotocol Greet (greet [this]))");
    assert!(
        r.symbols.iter().any(|s| s.name == "Greet" && s.kind == SymbolKind::Interface),
        "expected Interface Greet; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds — list_lit (call forms) and sym_lit (import targets)
// ---------------------------------------------------------------------------

/// list_lit non-declaration → Calls ref
#[test]
fn ref_list_lit_call() {
    let r = extract("(defn foo [x] (str/join x))");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "expected at least one Calls ref; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// sym_lit inside :require clause → Imports ref
#[test]
fn ref_sym_lit_imports() {
    let r = extract("(ns myapp.core (:require [clojure.string :as str]))");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports ref from :require; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}
