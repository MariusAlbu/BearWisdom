// =============================================================================
// go/flow_tests.rs — range-element binding for table-driven anonymous-struct
// slices. Drives the real Go extractor + flow pass and asserts the `range`
// value variable is narrowed to the enclosing function's qname (the anonymous
// struct's member-index key) over the loop body's byte range.
// =============================================================================

use crate::indexer::flow::run_flow_queries;
use crate::languages::go::{extract, GoPlugin};
use crate::languages::LanguagePlugin;
use crate::types::{FlowMeta, Narrowing};

/// Run the Go extractor + flow pass over `src` and return the `FlowMeta`.
fn flow_for(src: &str) -> FlowMeta {
    let result = extract::extract(src);
    let lang = GoPlugin.grammar("go").unwrap();
    let cfg = GoPlugin.flow_config().unwrap();
    let mut refs = result.refs;
    run_flow_queries(src, &lang, cfg, &result.symbols, &mut refs)
}

/// The narrowings recorded for the variable named `name`.
fn narrowings_for<'a>(meta: &'a FlowMeta, name: &str) -> Vec<&'a Narrowing> {
    meta.narrowings.iter().filter(|n| n.name == name).collect()
}

/// True when `[byte_start, byte_end)` covers the first byte of `needle` in `src`.
fn range_covers(n: &Narrowing, src: &str, needle: &str) -> bool {
    let Some(pos) = src.find(needle) else {
        return false;
    };
    let pos = pos as u32;
    n.byte_start <= pos && pos < n.byte_end
}

#[test]
fn range_value_var_narrowed_to_anon_struct_element() {
    // Canonical table-test shape: `tests := []struct{...}{...}` then
    // `for _, tc := range tests`. The value var `tc` is narrowed to the
    // enclosing-function qname over the loop body so `tc.expected` routes
    // through the function's anonymous-struct members.
    let src = r#"package core

func TestFind(t *T) {
	tests := []struct {
		name     string
		expected int
	}{
		{name: "a", expected: 1},
	}
	for _, tc := range tests {
		_ = tc.expected
	}
}
"#;
    let meta = flow_for(src);
    let ns = narrowings_for(&meta, "tc");
    assert_eq!(ns.len(), 1, "exactly one `tc` narrowing, got {ns:?}");
    assert_eq!(
        ns[0].narrowed_type, "core.TestFind",
        "tc narrows to the enclosing-function qname"
    );
    assert!(
        range_covers(ns[0], src, "tc.expected"),
        "the narrowing must cover the loop body where tc.expected is read"
    );
}

#[test]
fn collision_each_function_field_narrows_to_its_own_function() {
    // Field names recur across functions: both `TestA` and `TestB` declare a
    // field named `expected`. Each `tc` must narrow to ITS function's qname,
    // scoped to ITS loop body, so the byte ranges don't overlap and the reads
    // resolve to the right field symbol.
    let src = r#"package core

func TestA(t *T) {
	tests := []struct {
		expected int
	}{
		{expected: 1},
	}
	for _, tc := range tests {
		_ = tc.expected
	}
}

func TestB(t *T) {
	cases := []struct {
		expected string
	}{
		{expected: "x"},
	}
	for _, tc := range cases {
		_ = tc.expected
	}
}
"#;
    let meta = flow_for(src);
    let ns = narrowings_for(&meta, "tc");
    assert_eq!(ns.len(), 2, "one `tc` narrowing per function, got {ns:?}");
    let a = ns
        .iter()
        .find(|n| n.narrowed_type == "core.TestA")
        .expect("a narrowing to core.TestA");
    let b = ns
        .iter()
        .find(|n| n.narrowed_type == "core.TestB")
        .expect("a narrowing to core.TestB");
    // Distinct, non-overlapping body ranges — the collision guardrail.
    assert!(
        a.byte_end <= b.byte_start || b.byte_end <= a.byte_start,
        "the two `tc` narrowings must not overlap, got {a:?} and {b:?}"
    );
}

#[test]
fn named_struct_slice_range_narrows_to_the_type_qname() {
    // Ranging over a slice of a NAMED in-file struct narrows the value var to
    // the type's qualified name — never to the enclosing function's qname.
    let src = r#"package core

type Case struct {
	expected int
}

func TestNamed(t *T) {
	tests := []Case{
		{expected: 1},
	}
	for _, tc := range tests {
		_ = tc.expected
	}
}
"#;
    let meta = flow_for(src);
    let tc: Vec<_> = narrowings_for(&meta, "tc");
    assert!(
        tc.iter().any(|n| n.narrowed_type == "core.Case"),
        "named-struct slice range must narrow tc to core.Case, got {:?}",
        meta.narrowings
    );
    assert!(
        !tc.iter().any(|n| n.narrowed_type == "core.TestNamed"),
        "named-struct slice range must not narrow tc to the function qname, got {:?}",
        meta.narrowings
    );
}

#[test]
fn range_over_primitive_slice_unaffected() {
    // A non-struct range (`range nums` of ints) must not produce any
    // anon-struct narrowing for the value var.
    let src = r#"package core

func TestInts(t *T) {
	nums := []int{1, 2, 3}
	for _, n := range nums {
		_ = n
	}
}
"#;
    let meta = flow_for(src);
    assert!(
        narrowings_for(&meta, "n").is_empty(),
        "primitive-slice range must not narrow the value var, got {:?}",
        meta.narrowings
    );
}

#[test]
fn single_var_range_narrows_nothing() {
    // `for i := range tests` (single variable) iterates the INDEX, not the
    // element — `i` is an int. The element narrowing must not fire.
    let src = r#"package core

func TestSingle(t *T) {
	tests := []struct {
		expected int
	}{
		{expected: 1},
	}
	for i := range tests {
		_ = i
	}
}
"#;
    let meta = flow_for(src);
    assert!(
        narrowings_for(&meta, "i").is_empty(),
        "single-var range binds the index, not the struct element, got {:?}",
        meta.narrowings
    );
}
