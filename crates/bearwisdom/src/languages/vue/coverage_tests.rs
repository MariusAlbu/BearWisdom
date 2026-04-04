// =============================================================================
// vue/coverage_tests.rs
//
// Node-kind coverage for VuePlugin::symbol_node_kinds() and ref_node_kinds().
// symbol_node_kinds: script_element, template_element
// ref_node_kinds:    element, self_closing_tag, directive_attribute
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds: Class symbol inferred from filename
// ---------------------------------------------------------------------------

#[test]
fn cov_component_class_symbol_from_filename() {
    let r = extract::extract(
        "<template><div>Hello</div></template>",
        "MyButton.vue",
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "MyButton"),
        "Vue SFC should produce Class(MyButton) from filename; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds: element / self_closing_tag → Calls
// ---------------------------------------------------------------------------

#[test]
fn cov_element_pascal_produces_calls() {
    let r = extract::extract(
        "<template><UserCard></UserCard></template>",
        "App.vue",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "UserCard"),
        "PascalCase element should produce Calls(UserCard); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_self_closing_tag_produces_calls() {
    let r = extract::extract(
        "<template><Modal /></template>",
        "App.vue",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "Modal"),
        "self-closing PascalCase tag should produce Calls(Modal); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_kebab_element_produces_calls() {
    let r = extract::extract(
        "<template><my-component></my-component></template>",
        "Page.vue",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "MyComponent"),
        "kebab element should produce Calls(MyComponent); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
