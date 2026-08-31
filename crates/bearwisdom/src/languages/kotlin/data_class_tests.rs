// =============================================================================
// kotlin/data_class_tests.rs — unit tests for data-class member synthesis
// =============================================================================

use super::data_class::_test_synthesize;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Synthesized qualified names, sorted.
fn qnames(source: &str) -> Vec<String> {
    let mut v: Vec<String> = _test_synthesize(source)
        .symbols
        .into_iter()
        .map(|s| s.qualified_name)
        .collect();
    v.sort();
    v
}

/// Return-type ref target for the synthesized symbol with qualified name `qn`,
/// or `None` when the symbol emits no TypeRef.
/// `source_symbol_index` on synthesized refs is relative to the synthesized
/// symbol list, matching how parse_file rebases them.
fn return_ref_for(source: &str, qn: &str) -> Option<String> {
    let s = _test_synthesize(source);
    let idx = s.symbols.iter().position(|sy| sy.qualified_name == qn)?;
    s.refs
        .iter()
        .find(|rf| rf.source_symbol_index == idx && rf.kind == EdgeKind::TypeRef)
        .map(|rf| rf.target_name.clone())
}

// ---------------------------------------------------------------------------
// Plain class — nothing synthesized
// ---------------------------------------------------------------------------

#[test]
fn plain_class_yields_nothing() {
    assert!(
        _test_synthesize("class User(val name: String, val age: Int)")
            .symbols
            .is_empty(),
        "a plain (non-data) class must not trigger synthesis"
    );
}

#[test]
fn plain_class_with_data_in_name_yields_nothing() {
    // `database` contains "data" as a substring — must not match.
    assert!(
        _test_synthesize("class database(val host: String)")
            .symbols
            .is_empty(),
        "a class whose name contains 'data' as a substring must not trigger synthesis"
    );
}

#[test]
fn annotated_data_class_is_synthesized() {
    // The `class_declaration` node spans its annotations, so `data` is not on
    // the node's first line. Detection must scan to the `class` keyword.
    let src = "@Serializable\ndata class User(val id: Int)";
    let q = qnames(src);
    assert!(
        q.contains(&"User.copy".to_string()),
        "an annotated data class must still synthesize members; got {q:?}"
    );
}

// ---------------------------------------------------------------------------
// copy() synthesis
// ---------------------------------------------------------------------------

#[test]
fn copy_synthesized_for_data_class() {
    let q = qnames("data class User(val name: String, val age: Int)");
    assert!(
        q.contains(&"User.copy".to_string()),
        "copy() must be synthesized; got {q:?}"
    );
}

#[test]
fn copy_return_type_ref_points_to_class_qname() {
    // The return-type ref makes `u.copy()` type to User so `.name` chains through.
    assert_eq!(
        return_ref_for(
            "data class User(val name: String, val age: Int)",
            "User.copy"
        ),
        Some("User".to_string()),
        "copy() must carry a return-type ref to the class qname"
    );
}

#[test]
fn copy_return_type_ref_carries_package_qname() {
    let src = "package com.example\ndata class Point(val x: Int, val y: Int)";
    assert_eq!(
        return_ref_for(src, "com.example.Point.copy"),
        Some("com.example.Point".to_string()),
        "copy() return-type ref must use the fully-qualified class name"
    );
}

// ---------------------------------------------------------------------------
// componentN() synthesis
// ---------------------------------------------------------------------------

#[test]
fn component_methods_synthesized_in_order() {
    let q = qnames("data class User(val name: String, val age: Int)");
    assert!(
        q.contains(&"User.component1".to_string()),
        "component1 missing; got {q:?}"
    );
    assert!(
        q.contains(&"User.component2".to_string()),
        "component2 missing; got {q:?}"
    );
    assert!(
        !q.contains(&"User.component3".to_string()),
        "unexpected component3; got {q:?}"
    );
}

#[test]
fn component1_return_ref_is_string_type() {
    // `val name: String` → component1() returns String. String is a stdlib
    // scalar so no return-type ref is emitted (nothing to chain into).
    assert_eq!(
        return_ref_for(
            "data class User(val name: String, val age: Int)",
            "User.component1"
        ),
        None,
        "component1 on a String property must emit no return-type ref"
    );
}

#[test]
fn component_return_ref_emitted_for_non_primitive_type() {
    // `val address: Address` → component1() returns Address (non-primitive);
    // a return-type ref must be emitted so chains through component1() bind.
    let src = "data class Order(val address: Address, val count: Int)";
    assert_eq!(
        return_ref_for(src, "Order.component1"),
        Some("Address".to_string()),
        "component1 on a non-primitive type must emit a return-type ref"
    );
}

#[test]
fn prefix_named_property_resolves_its_own_type() {
    // `name` is a substring of the earlier `myname`; a scan from line start
    // would match `myname:` first and mistype component2. The column-anchored
    // scan must read each property's own declared type.
    let src = "data class Foo(val myname: Account, val name: User)";
    assert_eq!(
        return_ref_for(src, "Foo.component1"),
        Some("Account".to_string())
    );
    assert_eq!(
        return_ref_for(src, "Foo.component2"),
        Some("User".to_string())
    );
}

#[test]
fn plain_param_without_val_var_not_a_component() {
    // `class Greeter(name: String)` — no val/var, so `name` is NOT a
    // promoted property.  No componentN should be emitted.
    let q = qnames("data class Wrapper(x: Int)");
    assert!(
        !q.iter().any(|n| n.starts_with("Wrapper.component")),
        "a non-property ctor param must not become a componentN; got {q:?}"
    );
}

// ---------------------------------------------------------------------------
// equals / hashCode / toString
// ---------------------------------------------------------------------------

#[test]
fn structural_methods_synthesized() {
    let q = qnames("data class User(val name: String)");
    assert!(
        q.contains(&"User.equals".to_string()),
        "equals missing; got {q:?}"
    );
    assert!(
        q.contains(&"User.hashCode".to_string()),
        "hashCode missing; got {q:?}"
    );
    assert!(
        q.contains(&"User.toString".to_string()),
        "toString missing; got {q:?}"
    );
}

#[test]
fn structural_methods_emit_no_return_type_refs() {
    let src = "data class User(val name: String)";
    // Boolean / Int / String returns have no chain value — no refs expected.
    assert_eq!(return_ref_for(src, "User.equals"), None);
    assert_eq!(return_ref_for(src, "User.hashCode"), None);
    assert_eq!(return_ref_for(src, "User.toString"), None);
}

// ---------------------------------------------------------------------------
// Dedup: hand-written member wins
// ---------------------------------------------------------------------------

#[test]
fn hand_written_copy_not_duplicated() {
    // An explicit `fun copy(...)` in the class body must suppress synthesis.
    let src = "data class User(val name: String) {\n    fun copy(name: String = this.name): User = User(name)\n}";
    let q = qnames(src);
    let copies: Vec<_> = q.iter().filter(|n| n.ends_with(".copy")).collect();
    assert_eq!(
        copies.len(),
        0,
        "hand-written copy must not be duplicated; got {q:?}"
    );
}

#[test]
fn hand_written_component1_not_duplicated() {
    let src = "data class User(val name: String) {\n    fun component1(): String = name\n}";
    let q = qnames(src);
    let comps: Vec<_> = q.iter().filter(|n| n.ends_with(".component1")).collect();
    assert_eq!(
        comps.len(),
        0,
        "hand-written component1 must not be duplicated; got {q:?}"
    );
}

// ---------------------------------------------------------------------------
// Symbol kinds
// ---------------------------------------------------------------------------

#[test]
fn synthesized_members_have_method_kind() {
    let s = _test_synthesize("data class User(val name: String, val age: Int)");
    for sym in &s.symbols {
        assert_eq!(
            sym.kind,
            SymbolKind::Method,
            "synthesized member {} must be Method kind",
            sym.qualified_name
        );
    }
}

// ---------------------------------------------------------------------------
// Resolve / chain test: copy() return-type ref enables User.copy → User lookup
// ---------------------------------------------------------------------------

#[test]
fn copy_return_ref_enables_chain_resolution() {
    use crate::indexer::resolve::engine::compilation::Compilation;
    use crate::indexer::resolve::engine::contract::SymbolLookup;
    use crate::types::{EdgeKind, FlowMeta, ParsedFile};
    use std::collections::HashMap;

    // Build a merged symbol table (real + synthesized) and confirm that the
    // Compilation sees both User and User.copy, so the chain walker can follow
    // `u.copy()` → (return-type ref to User) → `.name`.
    let source = "data class User(val name: String, val age: Int)";
    let r = super::extract::extract(source);
    let synth = _test_synthesize(source);

    // copy() must exist with a return-type ref pointing at "User".
    let copy_idx = synth
        .symbols
        .iter()
        .position(|s| s.name == "copy")
        .expect("copy() must be synthesized");
    let copy_ref = synth
        .refs
        .iter()
        .find(|rf| rf.source_symbol_index == copy_idx && rf.kind == EdgeKind::TypeRef)
        .expect("copy() must carry a TypeRef");
    assert_eq!(copy_ref.target_name, "User");

    // Merge synthesized symbols/refs onto the extractor output (mirrors parse_file splice).
    let base = r.symbols.len();
    let mut all_symbols = r.symbols.clone();
    all_symbols.extend(synth.symbols.clone());
    let mut all_refs = r.refs.clone();
    for mut sref in synth.refs.clone() {
        sref.source_symbol_index += base;
        all_refs.push(sref);
    }

    let pf = ParsedFile {
        path: "src/User.kt".to_string(),
        language: "kotlin".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 1,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: all_symbols.clone(),
        refs: all_refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    };

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    for (i, sym) in all_symbols.iter().enumerate() {
        id_map.insert(
            ("src/User.kt".to_string(), sym.qualified_name.clone()),
            i as i64 + 1,
        );
    }

    let index = Compilation::build(&[pf], &id_map.clone().into(), std::sync::Arc::new(crate::type_checker::core::types::TypeArena::new()));

    // Both User and User.copy must be reachable in the index.
    assert!(
        !index.by_name("User").is_empty(),
        "User must be in the index"
    );
    assert!(
        index.by_qualified_name("User.copy").is_some(),
        "User.copy must be in the index after merging synthesized symbols"
    );

    // User.name must also be reachable as a member of User, confirming the
    // chain walker can reach it after following copy()'s return-type ref.
    let user_members = index.members_of("User");
    assert!(
        user_members.iter().any(|si| si.name == "name"),
        "User.name must be a member of User in the index so .copy().name chains through"
    );
}
