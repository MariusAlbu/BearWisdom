// =============================================================================
// rust_lang/derives_tests.rs — unit tests for #[derive(...)] member synthesis
// =============================================================================

use super::derives::_test_synthesize;
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
/// or `None` when the symbol emits no TypeRef. `source_symbol_index` on
/// synthesized refs is relative to the synthesized symbol list, matching how
/// parse_file rebases them.
fn return_ref_for(source: &str, qn: &str) -> Option<String> {
    let s = _test_synthesize(source);
    let idx = s.symbols.iter().position(|sy| sy.qualified_name == qn)?;
    s.refs
        .iter()
        .find(|rf| rf.source_symbol_index == idx && rf.kind == EdgeKind::TypeRef)
        .map(|rf| rf.target_name.clone())
}

// ---------------------------------------------------------------------------
// No derive — nothing synthesized
// ---------------------------------------------------------------------------

#[test]
fn struct_without_derive_yields_nothing() {
    assert!(
        _test_synthesize("struct Config { name: String }")
            .symbols
            .is_empty(),
        "a struct without #[derive(...)] must not trigger synthesis"
    );
}

// ---------------------------------------------------------------------------
// Self-returning derives carry a return-type ref to the owning type — the
// headline gap: the inline synth emitted no ref, so `.default()`/`.clone()`
// did not type through.
// ---------------------------------------------------------------------------

#[test]
fn default_synthesized_with_self_return_ref() {
    let src = "#[derive(Default)]\nstruct Config { name: String }";
    let q = qnames(src);
    assert!(
        q.contains(&"Config.default".to_string()),
        "default() must be synthesized; got {q:?}"
    );
    assert_eq!(
        return_ref_for(src, "Config.default"),
        Some("Config".to_string()),
        "default() must carry a return-type ref to the owning struct qname"
    );
}

#[test]
fn clone_synthesized_with_self_return_ref() {
    let src = "#[derive(Clone)]\nstruct User { id: u32 }";
    let q = qnames(src);
    assert!(
        q.contains(&"User.clone".to_string()),
        "clone() must be synthesized; got {q:?}"
    );
    assert_eq!(
        return_ref_for(src, "User.clone"),
        Some("User".to_string()),
        "clone() must carry a return-type ref to the owning struct qname"
    );
}

#[test]
fn from_synthesized_with_self_return_ref() {
    let src = "#[derive(From)]\nstruct Wrapper { inner: u32 }";
    assert_eq!(
        return_ref_for(src, "Wrapper.from"),
        Some("Wrapper".to_string()),
        "from() must carry a return-type ref to the owning struct qname"
    );
}

#[test]
fn scoped_derive_path_recognized() {
    // `serde::Serialize` arrives as a scoped path; the bare tail must drive
    // recognition. Serialize synthesizes `serialize` (no Self return).
    let src = "#[derive(serde::Serialize)]\nstruct Payload { id: u32 }";
    let q = qnames(src);
    assert!(
        q.contains(&"Payload.serialize".to_string()),
        "serde::Serialize must synthesize serialize(); got {q:?}"
    );
}

// ---------------------------------------------------------------------------
// Non-Self returns emit no return-type ref (primitive-skip discipline).
// ---------------------------------------------------------------------------

#[test]
fn non_self_returns_emit_no_return_ref() {
    // Debug→fmt (Result), PartialEq→eq/ne (bool), Hash→hash (()),
    // Ord→cmp (Ordering): none is the owning type, so no chain value.
    let src = "#[derive(Debug, PartialEq, Hash, Ord)]\nstruct K { id: u32 }";
    assert_eq!(return_ref_for(src, "K.fmt"), None);
    assert_eq!(return_ref_for(src, "K.eq"), None);
    assert_eq!(return_ref_for(src, "K.hash"), None);
    assert_eq!(return_ref_for(src, "K.cmp"), None);
}

// ---------------------------------------------------------------------------
// Enums derive the same way.
// ---------------------------------------------------------------------------

#[test]
fn enum_default_synthesized_with_self_return_ref() {
    let src = "#[derive(Default, Clone)]\nenum Mode { #[default] A, B }";
    assert_eq!(
        return_ref_for(src, "Mode.default"),
        Some("Mode".to_string()),
        "enum default() must carry a return-type ref to the enum qname"
    );
    assert_eq!(
        return_ref_for(src, "Mode.clone"),
        Some("Mode".to_string()),
        "enum clone() must carry a return-type ref to the enum qname"
    );
}

// ---------------------------------------------------------------------------
// Dedup: a hand-written member of the same qname wins.
// ---------------------------------------------------------------------------

#[test]
fn hand_written_member_wins() {
    // An explicit `fn clone(&self) -> Self` in an impl block must suppress the
    // synthesized Config.clone — no duplicate.
    let src = "#[derive(Clone)]\nstruct Config { name: String }\nimpl Config {\n    fn clone(&self) -> Self { Config { name: self.name.clone() } }\n}";
    let q = qnames(src);
    let clones: Vec<_> = q.iter().filter(|n| n.as_str() == "Config.clone").collect();
    assert_eq!(
        clones.len(),
        0,
        "hand-written clone must not be duplicated by synthesis; got {q:?}"
    );
}

// ---------------------------------------------------------------------------
// Symbol kinds: synthesized members are Method/Function, never type kinds.
// ---------------------------------------------------------------------------

#[test]
fn synthesized_members_are_callable_kinds() {
    let s = _test_synthesize("#[derive(Clone, Default)]\nstruct User { id: u32 }");
    for sym in &s.symbols {
        assert!(
            matches!(sym.kind, SymbolKind::Method | SymbolKind::Function),
            "synthesized member {} must be a callable kind, got {:?}",
            sym.qualified_name,
            sym.kind
        );
    }
}

// ---------------------------------------------------------------------------
// e2e: default()'s return-type ref binds at the index level so
// `Config::default().name` resolves the field through the synthesized method.
// ---------------------------------------------------------------------------

#[test]
fn default_chains_through_at_index_level() {
    use crate::indexer::resolve::engine::compilation::Compilation;
    use crate::indexer::resolve::engine::contract::SymbolLookup;
    use crate::types::{FlowMeta, ParsedFile};
    use std::collections::HashMap;

    let source = "#[derive(Default)]\nstruct Config { name: String }";
    let r = super::extract::extract(source);
    let synth = _test_synthesize(source);

    // default() must exist with a return-type ref pointing at "Config".
    let default_idx = synth
        .symbols
        .iter()
        .position(|s| s.name == "default")
        .expect("default() must be synthesized");
    let default_ref = synth
        .refs
        .iter()
        .find(|rf| rf.source_symbol_index == default_idx && rf.kind == EdgeKind::TypeRef)
        .expect("default() must carry a TypeRef");
    assert_eq!(default_ref.target_name, "Config");

    // Merge synthesized symbols/refs onto the extractor output, rebasing
    // source_symbol_index by the pre-append symbol count (mirrors parse_file).
    let base = r.symbols.len();
    let mut all_symbols = r.symbols.clone();
    all_symbols.extend(synth.symbols.clone());
    let mut all_refs = r.refs.clone();
    for mut sref in synth.refs.clone() {
        sref.source_symbol_index += base;
        all_refs.push(sref);
    }

    let pf = ParsedFile {
        path: "src/config.rs".to_string(),
        language: "rust".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 2,
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
            ("src/config.rs".to_string(), sym.qualified_name.clone()),
            i as i64 + 1,
        );
    }

    let index = Compilation::build(&[pf], &id_map.clone().into(), std::sync::Arc::new(crate::type_checker::core::types::TypeArena::new()));

    // Config.default must be reachable with return_type == "Config".
    assert!(
        index.by_qualified_name("Config.default").is_some(),
        "Config.default must be in the index after merging synthesized symbols"
    );
    assert_eq!(
        index.return_type_str("Config.default").as_deref(),
        Some("Config"),
        "Config.default's return type must resolve to Config so .default().name chains through"
    );

    // Config.name must be a member of Config, the chain target after default().
    let members = index.members_of("Config");
    assert!(
        members.iter().any(|si| si.name == "name"),
        "Config.name must be a member of Config so Config::default().name chains through"
    );
}
