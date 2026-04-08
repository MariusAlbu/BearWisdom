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

// ---------------------------------------------------------------------------
// directive_attribute — v-on:event and @event shorthand → Calls(handler)
// ---------------------------------------------------------------------------

#[test]
fn cov_directive_attribute_v_on_produces_calls() {
    // directive_attribute: v-on:click="submitForm" → Calls(submitForm)
    let r = extract::extract(
        r#"<template><form v-on:submit="submitForm"></form></template>"#,
        "Form.vue",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "submitForm"),
        "v-on:submit directive should produce Calls(submitForm); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_directive_attribute_at_shorthand_produces_calls() {
    // @click="handler" shorthand form → Calls(handler)
    let r = extract::extract(
        r#"<template><button @click="handleClick">Click</button></template>"#,
        "Button.vue",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "handleClick"),
        "@click shorthand should produce Calls(handleClick); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_directive_attribute_handler_with_args_produces_calls() {
    // @click="handler($event)" — strip args to get bare handler name
    let r = extract::extract(
        r#"<template><input @input="onInput($event)" /></template>"#,
        "Input.vue",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "onInput"),
        "@input with $event arg should produce Calls(onInput); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// standard HTML — no spurious Calls edges for native elements
// ---------------------------------------------------------------------------

#[test]
fn cov_standard_html_elements_do_not_produce_calls() {
    // Lowercase HTML tags inside <template> must NOT produce Calls edges
    let r = extract::extract(
        "<template><div><p><span>text</span></p></div></template>",
        "Layout.vue",
    );
    let html_calls: Vec<_> = r.refs.iter()
        .filter(|rf| rf.kind == EdgeKind::Calls)
        .filter(|rf| matches!(rf.target_name.as_str(), "div" | "Div" | "p" | "P" | "span" | "Span"))
        .collect();
    assert!(
        html_calls.is_empty(),
        "standard HTML elements must not produce Calls; got: {:?}",
        html_calls.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// component Class symbol — correct name from various filename patterns
// ---------------------------------------------------------------------------

#[test]
fn cov_component_symbol_from_multi_word_filename() {
    // UserProfileCard.vue → Class(UserProfileCard)
    let r = extract::extract(
        "<template><div></div></template>",
        "UserProfileCard.vue",
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "UserProfileCard"),
        "multi-word filename should produce Class(UserProfileCard); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// nested component in template body
// ---------------------------------------------------------------------------

#[test]
fn cov_nested_pascal_component_in_template_body_produces_calls() {
    // Component nested inside native elements still produces a Calls edge
    let r = extract::extract(
        "<template><div><section><DataTable /></section></div></template>",
        "Dashboard.vue",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "DataTable"),
        "nested PascalCase component should produce Calls(DataTable); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
