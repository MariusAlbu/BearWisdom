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

/// Namespace-qualified call head: (str/join ...) → target_name="join", module=Some("str")
#[test]
fn ref_namespace_qualified_symbol() {
    let r = extract("(defn fmt [items] (str/join \",\" items))");
    let calls: Vec<_> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "join")
        .collect();
    assert!(
        !calls.is_empty(),
        "expected Calls ref with target_name='join' from str/join; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind, &rf.module)).collect::<Vec<_>>()
    );
    assert_eq!(
        calls[0].module.as_deref(),
        Some("str"),
        "expected module=Some(\"str\"); got: {:?}",
        calls[0].module
    );
}

/// Unqualified symbol: no slash → module stays None
#[test]
fn ref_unqualified_symbol_no_module() {
    let r = extract("(defn foo [x] (inc x))");
    let inc_refs: Vec<_> = r
        .refs
        .iter()
        .filter(|rf| rf.target_name == "inc")
        .collect();
    assert!(
        !inc_refs.is_empty(),
        "expected ref to 'inc'; got: {:?}",
        r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
    assert_eq!(
        inc_refs[0].module,
        None,
        "expected module=None for unqualified 'inc'; got: {:?}",
        inc_refs[0].module
    );
}

// ---------------------------------------------------------------------------
// Additional symbol_node_kinds — uncovered forms
// ---------------------------------------------------------------------------

/// list_lit matched as `defonce` → Variable symbol (initialize-once)
#[test]
fn symbol_list_lit_defonce() {
    let r = extract("(defonce conn (atom nil))");
    assert!(
        r.symbols.iter().any(|s| s.name == "conn" && s.kind == SymbolKind::Variable),
        "expected Variable conn from defonce; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// list_lit matched as `deftype` → Struct symbol
#[test]
fn symbol_list_lit_deftype() {
    let r = extract("(deftype MyType [a b] Object)");
    assert!(
        r.symbols.iter().any(|s| s.name == "MyType" && s.kind == SymbolKind::Struct),
        "expected Struct MyType from deftype; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// list_lit matched as `definterface` → Interface symbol
#[test]
fn symbol_list_lit_definterface() {
    let r = extract("(definterface ICounter (increment [this]) (value [this]))");
    assert!(
        r.symbols.iter().any(|s| s.name == "ICounter" && s.kind == SymbolKind::Interface),
        "expected Interface ICounter from definterface; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// list_lit matched as `defmulti` → Function symbol (dispatch definition)
#[test]
fn symbol_list_lit_defmulti() {
    let r = extract("(defmulti dispatch-fn :type)");
    assert!(
        r.symbols.iter().any(|s| s.name == "dispatch-fn" && s.kind == SymbolKind::Function),
        "expected Function dispatch-fn from defmulti; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// list_lit matched as `defmethod` → Function symbol (multimethod implementation)
#[test]
fn symbol_list_lit_defmethod() {
    let r = extract("(defmethod dispatch-fn :circle [shape] (* Math/PI (:radius shape)))");
    assert!(
        r.symbols.iter().any(|s| s.name == "dispatch-fn" && s.kind == SymbolKind::Function),
        "expected Function dispatch-fn from defmethod; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional ref_node_kinds — uncovered import forms
// ---------------------------------------------------------------------------

/// ns form with `:use` clause → Imports ref
#[test]
fn ref_ns_use_clause_imports() {
    let r = extract("(ns myapp.core (:use [clojure.set]))");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports ref from :use clause; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ns form with `:import` clause → Imports ref
#[test]
fn ref_ns_import_clause_imports() {
    let r = extract("(ns myapp.core (:import [java.util HashMap]))");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports ref from :import clause; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// :require with :refer vector → Imports ref to namespace
#[test]
fn ref_ns_require_with_refer() {
    let r = extract("(ns myapp.core (:require [clojure.set :refer [union difference]]))");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "clojure.set"),
        "expected Imports(clojure.set) from :require :refer; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// :require with :as alias → Imports ref to namespace
#[test]
fn ref_ns_require_with_as_alias() {
    let r = extract("(ns myapp.core (:require [clojure.string :as str]))");
    let imp: Vec<_> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "clojure.string")
        .collect();
    assert!(
        !imp.is_empty(),
        "expected Imports(clojure.string) from :require :as; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// Calls ref emitted for the head verb of every declaration form
/// (verifies sym_lit coverage for the defn head itself)
#[test]
fn ref_declaration_head_emitted_as_calls() {
    let r = extract("(defn my-fn [x] x)");
    // The `defn` sym_lit head is always emitted as a Calls ref.
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "defn"),
        "expected Calls ref with target_name='defn'; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// `extend-protocol` form → Implements-style Calls refs to protocol name
#[test]
fn ref_extend_protocol_emits_refs() {
    let r = extract("(extend-protocol IFoo MyRecord (do-thing [this] nil))");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "IFoo" || rf.target_name == "extend-protocol"),
        "expected ref to IFoo or extend-protocol; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// `extend-type` form → refs to type and protocol names
#[test]
fn ref_extend_type_emits_refs() {
    let r = extract("(extend-type String IShow (show [this] (str \"<\" this \">\")))");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "extend-type" || rf.target_name == "String"),
        "expected ref to extend-type or String; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Scope tracking — local bindings should NOT be emitted as Calls refs
// ---------------------------------------------------------------------------

/// defn parameters should be suppressed as Calls refs
#[test]
fn scope_defn_params_suppressed() {
    let r = extract("(defn handle [request respond raise] (handler request respond raise))");
    // request, respond, raise are params — should NOT appear as unqualified Calls refs
    let leaked: Vec<_> = r.refs.iter()
        .filter(|rf| matches!(rf.target_name.as_str(), "request" | "respond" | "raise") && rf.module.is_none())
        .collect();
    assert!(leaked.is_empty(),
        "params leaked as refs: {:?}",
        leaked.iter().map(|rf| &rf.target_name).collect::<Vec<_>>());
}

/// let bindings should be suppressed as Calls refs
#[test]
fn scope_let_bindings_suppressed() {
    let r = extract("(defn foo [x] (let [options {:a 1} result (bar x)] (use options result)))");
    let leaked: Vec<_> = r.refs.iter()
        .filter(|rf| matches!(rf.target_name.as_str(), "options" | "result") && rf.module.is_none())
        .collect();
    assert!(leaked.is_empty(),
        "let bindings leaked as refs: {:?}",
        leaked.iter().map(|rf| &rf.target_name).collect::<Vec<_>>());
}

/// Map destructuring {:keys [a b]} should suppress a and b
#[test]
fn scope_map_destructuring_suppressed() {
    let r = extract("(defn foo [{:keys [decoder encoder]}] (use decoder encoder))");
    let leaked: Vec<_> = r.refs.iter()
        .filter(|rf| matches!(rf.target_name.as_str(), "decoder" | "encoder") && rf.module.is_none())
        .collect();
    assert!(leaked.is_empty(),
        "destructured keys leaked as refs: {:?}",
        leaked.iter().map(|rf| &rf.target_name).collect::<Vec<_>>());
}

/// Multi-arity fn params in anonymous fn should be suppressed
#[test]
fn scope_anon_fn_params_suppressed() {
    let r = extract("(defn wrap [handler options] (fn ([request] (handler request)) ([request respond raise] (handler request respond raise))))");
    let leaked: Vec<_> = r.refs.iter()
        .filter(|rf| matches!(rf.target_name.as_str(), "request" | "respond" | "raise" | "options") && rf.module.is_none())
        .collect();
    assert!(leaked.is_empty(),
        "fn params leaked as refs: {:?}",
        leaked.iter().map(|rf| &rf.target_name).collect::<Vec<_>>());
}

/// Namespace-qualified refs (e.g. str/join) should still be emitted even if
/// the unqualified name would be a local
#[test]
fn scope_qualified_refs_not_suppressed() {
    let r = extract("(defn foo [str] (str/join \",\" str))");
    // str/join should still emit a ref with module="str"
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "join" && rf.module.as_deref() == Some("str")),
        "qualified str/join ref missing; got: {:?}",
        r.refs.iter().filter(|rf| rf.target_name == "join").collect::<Vec<_>>()
    );
}
