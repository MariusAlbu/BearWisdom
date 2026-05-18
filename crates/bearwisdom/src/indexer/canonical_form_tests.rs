use super::*;
use crate::types::{
    CallArg, ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta,
    MemberChain, ParsedFile, SegmentKind, SymbolKind, Visibility,
};

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn make_pf(symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: "src/file.rs".to_string(),
        language: "rust".to_string(),
        content_hash: "h".to_string(),
        size: 1024,
        line_count: 10,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn make_sym(name: &str, qname: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 5,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn make_ref(source_idx: usize, target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind,
        line: 3,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 42,
        call_args: Vec::new(),
    }
}

fn seg(name: &str, kind: SegmentKind) -> ChainSegment {
    ChainSegment {
        name: name.to_string(),
        node_kind: "identifier".to_string(),
        kind,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
    }
}

fn codes(violations: &[ContractViolation]) -> Vec<&'static str> {
    violations.iter().map(|v| v.code).collect()
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[test]
fn clean_file_produces_no_violations() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let r = make_ref(0, "bar", EdgeKind::Calls);
    let pf = make_pf(vec![sym], vec![r]);
    assert!(validate(&pf).is_empty());
}

#[test]
fn nested_symbol_with_parent_is_clean() {
    let parent = make_sym("Foo", "ns::Foo", SymbolKind::Class);
    let mut child = make_sym("bar", "ns::Foo::bar", SymbolKind::Method);
    child.parent_index = Some(0);
    child.scope_path = Some("ns::Foo".to_string());
    let pf = make_pf(vec![parent, child], Vec::new());
    let v = validate(&pf);
    assert!(v.is_empty(), "unexpected violations: {v:?}");
}

#[test]
fn dotted_call_with_chain_is_clean() {
    let sym = make_sym("caller", "caller", SymbolKind::Function);
    let mut r = make_ref(0, "save", EdgeKind::Calls);
    r.chain = Some(MemberChain {
        segments: vec![
            seg("repo", SegmentKind::Identifier),
            seg("save", SegmentKind::Property),
        ],
    });
    let pf = make_pf(vec![sym], vec![r]);
    let v = validate(&pf);
    assert!(v.is_empty(), "unexpected violations: {v:?}");
}

// ---------------------------------------------------------------------------
// SYM-001
// ---------------------------------------------------------------------------

#[test]
fn sym_001_flags_qname_that_does_not_end_with_name() {
    let sym = make_sym("Bar", "Foo.Baz", SymbolKind::Class);
    let pf = make_pf(vec![sym], Vec::new());
    assert_eq!(codes(&validate(&pf)), vec!["SYM-001"]);
}

#[test]
fn sym_001_flags_missing_separator_before_name() {
    // qname ends with name but there's no separator char in front of it.
    let sym = make_sym("Bar", "FooBar", SymbolKind::Class);
    let pf = make_pf(vec![sym], Vec::new());
    assert_eq!(codes(&validate(&pf)), vec!["SYM-001"]);
}

#[test]
fn sym_001_accepts_top_level_symbol() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let pf = make_pf(vec![sym], Vec::new());
    assert!(validate(&pf).is_empty());
}

#[test]
fn sym_001_accepts_known_separators() {
    for qname in ["a.b.Foo", "a::b::Foo", "a/b/Foo", "a\\b\\Foo", "pkg$Foo"] {
        let sym = make_sym("Foo", qname, SymbolKind::Class);
        let pf = make_pf(vec![sym], Vec::new());
        let v = validate(&pf);
        assert!(v.is_empty(), "qname '{qname}' should be accepted, got {v:?}");
    }
}

// ---------------------------------------------------------------------------
// SYM-002
// ---------------------------------------------------------------------------

#[test]
fn sym_002_flags_scope_path_disagreement() {
    let parent = make_sym("Foo", "ns.Foo", SymbolKind::Class);
    let mut child = make_sym("bar", "ns.Foo.bar", SymbolKind::Method);
    child.parent_index = Some(0);
    child.scope_path = Some("wrong.path".to_string());
    let pf = make_pf(vec![parent, child], Vec::new());
    assert_eq!(codes(&validate(&pf)), vec!["SYM-002"]);
}

#[test]
fn sym_002_flags_missing_scope_path_when_parent_set() {
    let parent = make_sym("Foo", "ns.Foo", SymbolKind::Class);
    let mut child = make_sym("bar", "ns.Foo.bar", SymbolKind::Method);
    child.parent_index = Some(0);
    // scope_path left as None.
    let pf = make_pf(vec![parent, child], Vec::new());
    assert_eq!(codes(&validate(&pf)), vec!["SYM-002"]);
}

// ---------------------------------------------------------------------------
// SYM-003
// ---------------------------------------------------------------------------

#[test]
fn sym_003_flags_self_referential_parent_index() {
    let mut sym = make_sym("foo", "foo", SymbolKind::Function);
    sym.parent_index = Some(0);
    let pf = make_pf(vec![sym], Vec::new());
    // 0 >= 0 → violation, plus SYM-002 because scope_path is missing.
    let v = codes(&validate(&pf));
    assert!(v.contains(&"SYM-003"), "expected SYM-003 in {v:?}");
}

#[test]
fn sym_003_flags_forward_parent_index() {
    let mut a = make_sym("a", "a", SymbolKind::Class);
    a.parent_index = Some(1); // forward reference is forbidden
    let b = make_sym("b", "b", SymbolKind::Class);
    let pf = make_pf(vec![a, b], Vec::new());
    let v = codes(&validate(&pf));
    assert!(v.contains(&"SYM-003"), "expected SYM-003 in {v:?}");
}

// ---------------------------------------------------------------------------
// REF-001
// ---------------------------------------------------------------------------

#[test]
fn ref_001_flags_out_of_bounds_source_symbol_index() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let r = make_ref(5, "bar", EdgeKind::Calls); // only 1 symbol
    let pf = make_pf(vec![sym], vec![r]);
    assert_eq!(codes(&validate(&pf)), vec!["REF-001"]);
}

// ---------------------------------------------------------------------------
// REF-002
// ---------------------------------------------------------------------------

#[test]
fn ref_002_flags_zero_byte_offset_on_calls_ref() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let mut r = make_ref(0, "bar", EdgeKind::Calls);
    r.byte_offset = 0;
    r.line = 10;
    let pf = make_pf(vec![sym], vec![r]);
    assert_eq!(codes(&validate(&pf)), vec!["REF-002"]);
}

#[test]
fn ref_002_flags_zero_byte_offset_on_typeref() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let mut r = make_ref(0, "Bar", EdgeKind::TypeRef);
    r.byte_offset = 0;
    r.line = 10;
    let pf = make_pf(vec![sym], vec![r]);
    assert_eq!(codes(&validate(&pf)), vec!["REF-002"]);
}

#[test]
fn ref_002_flags_zero_byte_offset_on_imports() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let mut r = make_ref(0, "lodash", EdgeKind::Imports);
    r.byte_offset = 0;
    r.line = 5;
    let pf = make_pf(vec![sym], vec![r]);
    assert_eq!(codes(&validate(&pf)), vec!["REF-002"]);
}

#[test]
fn ref_002_tolerates_zero_byte_offset_at_file_start() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let mut r = make_ref(0, "bar", EdgeKind::Calls);
    r.byte_offset = 0;
    r.line = 0;
    let pf = make_pf(vec![sym], vec![r]);
    assert!(validate(&pf).is_empty());
}

#[test]
fn ref_002_tolerates_zero_byte_in_empty_file() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let mut r = make_ref(0, "bar", EdgeKind::Calls);
    r.byte_offset = 0;
    r.line = 7;
    let mut pf = make_pf(vec![sym], vec![r]);
    pf.size = 0;
    assert!(validate(&pf).is_empty());
}

// ---------------------------------------------------------------------------
// REF-003
// ---------------------------------------------------------------------------

#[test]
fn ref_003_flags_dotted_call_target_with_no_chain() {
    let sym = make_sym("caller", "caller", SymbolKind::Function);
    let r = make_ref(0, "repo.save", EdgeKind::Calls); // dotted, no chain
    let pf = make_pf(vec![sym], vec![r]);
    let v = codes(&validate(&pf));
    assert!(v.contains(&"REF-003"), "expected REF-003 in {v:?}");
}

#[test]
fn ref_003_allows_bare_call_with_no_chain() {
    let sym = make_sym("caller", "caller", SymbolKind::Function);
    let r = make_ref(0, "save", EdgeKind::Calls);
    let pf = make_pf(vec![sym], vec![r]);
    assert!(validate(&pf).is_empty());
}

// ---------------------------------------------------------------------------
// REF-004
// ---------------------------------------------------------------------------

#[test]
fn ref_004_flags_last_segment_mismatch_with_target_name() {
    let sym = make_sym("caller", "caller", SymbolKind::Function);
    let mut r = make_ref(0, "save", EdgeKind::Calls);
    r.chain = Some(MemberChain {
        segments: vec![
            seg("repo", SegmentKind::Identifier),
            seg("delete", SegmentKind::Property), // doesn't match target_name "save"
        ],
    });
    let pf = make_pf(vec![sym], vec![r]);
    assert_eq!(codes(&validate(&pf)), vec!["REF-004"]);
}

// ---------------------------------------------------------------------------
// REF-005
// ---------------------------------------------------------------------------

#[test]
fn ref_005_flags_call_args_on_typeref() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let mut r = make_ref(0, "Foo", EdgeKind::TypeRef);
    r.call_args.push(CallArg::StringLit("hi".to_string()));
    let pf = make_pf(vec![sym], vec![r]);
    assert_eq!(codes(&validate(&pf)), vec!["REF-005"]);
}

#[test]
fn ref_005_allows_call_args_on_calls() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let mut r = make_ref(0, "log", EdgeKind::Calls);
    r.call_args.push(CallArg::StringLit("hi".to_string()));
    let pf = make_pf(vec![sym], vec![r]);
    assert!(validate(&pf).is_empty());
}

// ---------------------------------------------------------------------------
// CHAIN-001 / CHAIN-002
// ---------------------------------------------------------------------------

#[test]
fn chain_001_flags_empty_segments() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let mut r = make_ref(0, "bar", EdgeKind::Calls);
    r.chain = Some(MemberChain { segments: Vec::new() });
    let pf = make_pf(vec![sym], vec![r]);
    let v = codes(&validate(&pf));
    assert!(v.contains(&"CHAIN-001"), "expected CHAIN-001 in {v:?}");
}

#[test]
fn chain_002_flags_property_first_segment() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let mut r = make_ref(0, "bar", EdgeKind::Calls);
    r.chain = Some(MemberChain {
        segments: vec![
            seg("bar", SegmentKind::Property), // can't START with Property
        ],
    });
    let pf = make_pf(vec![sym], vec![r]);
    let v = codes(&validate(&pf));
    assert!(v.contains(&"CHAIN-002"), "expected CHAIN-002 in {v:?}");
}

#[test]
fn chain_002_flags_computed_first_segment() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let mut r = make_ref(0, "bar", EdgeKind::Calls);
    r.chain = Some(MemberChain {
        segments: vec![
            seg("bar", SegmentKind::ComputedAccess),
        ],
    });
    let pf = make_pf(vec![sym], vec![r]);
    let v = codes(&validate(&pf));
    assert!(v.contains(&"CHAIN-002"), "expected CHAIN-002 in {v:?}");
}

// ---------------------------------------------------------------------------
// FILE-* rules
// ---------------------------------------------------------------------------

#[test]
fn file_001_flags_parallel_vec_length_mismatch() {
    let pf = ParsedFile {
        symbol_origin_languages: vec![None, None], // 2 entries but no symbols
        ..make_pf(Vec::new(), Vec::new())
    };
    assert_eq!(codes(&validate(&pf)), vec!["FILE-001"]);
}

#[test]
fn file_002_flags_ref_origin_length_mismatch() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let r = make_ref(0, "bar", EdgeKind::Calls);
    let pf = ParsedFile {
        ref_origin_languages: vec![None, None, None],
        ..make_pf(vec![sym], vec![r])
    };
    assert_eq!(codes(&validate(&pf)), vec!["FILE-002"]);
}

#[test]
fn file_003_flags_snippet_flag_length_mismatch() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let pf = ParsedFile {
        symbol_from_snippet: vec![false, true, false],
        ..make_pf(vec![sym], Vec::new())
    };
    assert_eq!(codes(&validate(&pf)), vec!["FILE-003"]);
}

#[test]
fn file_004_flags_invalid_flow_binding_key() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let r = make_ref(0, "bar", EdgeKind::Calls);
    let mut pf = make_pf(vec![sym], vec![r]);
    pf.flow.flow_binding_lhs.insert(99, 0); // ref index 99 doesn't exist
    assert_eq!(codes(&validate(&pf)), vec!["FILE-004"]);
}

#[test]
fn file_004_flags_invalid_flow_binding_value() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let r = make_ref(0, "bar", EdgeKind::Calls);
    let mut pf = make_pf(vec![sym], vec![r]);
    pf.flow.flow_binding_lhs.insert(0, 99); // symbol index 99 doesn't exist
    assert_eq!(codes(&validate(&pf)), vec!["FILE-004"]);
}

// ---------------------------------------------------------------------------
// assert_canonical
// ---------------------------------------------------------------------------

#[test]
fn assert_canonical_succeeds_on_clean_file() {
    let sym = make_sym("foo", "foo", SymbolKind::Function);
    let pf = make_pf(vec![sym], Vec::new());
    assert_canonical(&pf); // must not panic
}

#[test]
#[should_panic(expected = "canonical-form contract violations")]
fn assert_canonical_panics_on_violation() {
    let sym = make_sym("Bar", "Foo.Baz", SymbolKind::Class); // SYM-001
    let pf = make_pf(vec![sym], Vec::new());
    assert_canonical(&pf);
}
