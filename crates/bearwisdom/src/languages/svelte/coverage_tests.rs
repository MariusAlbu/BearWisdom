// =============================================================================
// svelte/coverage_tests.rs
//
// Node-kind coverage for SveltePlugin::symbol_node_kinds() and ref_node_kinds().
// symbol_node_kinds: script_element, element
// ref_node_kinds:    element, self_closing_element, attribute
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds: component Class symbol from filename
// ---------------------------------------------------------------------------

#[test]
fn cov_component_class_symbol_from_filename() {
    // script_element / element presence → Class symbol named from filename stem
    let r = extract::extract(
        "<script>\nlet count = 0;\n</script>\n<button>{count}</button>",
        "Counter.svelte",
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "Counter"),
        "Svelte component should produce Class(Counter); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds: element / self_closing_element → Calls
// ---------------------------------------------------------------------------

#[test]
fn cov_pascal_case_element_produces_calls() {
    // "element" with PascalCase tag → Calls (ref_node_kinds: element)
    let r = extract::extract("<UserCard></UserCard>", "App.svelte");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "UserCard"),
        "PascalCase element should produce Calls(UserCard); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_self_closing_element_produces_calls() {
    let r = extract::extract("<Modal />", "Page.svelte");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "Modal"),
        "self-closing PascalCase element should produce Calls(Modal); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_kebab_element_produces_calls() {
    // Kebab-case custom element → normalised PascalCase Calls
    let r = extract::extract("<my-widget></my-widget>", "Page.svelte");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "MyWidget"),
        "kebab element should produce Calls(MyWidget); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
