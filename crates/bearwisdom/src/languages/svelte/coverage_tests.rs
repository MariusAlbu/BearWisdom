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

// ---------------------------------------------------------------------------
// attribute (on:event) — Svelte event directive → Calls(handler)
// ---------------------------------------------------------------------------

#[test]
fn cov_on_event_attribute_quoted_produces_calls() {
    // on:click="handler" with quoted string value → Calls(handler)
    let r = extract::extract(
        r#"<button on:click="handleClick">Click</button>"#,
        "Button.svelte",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "handleClick"),
        "on:click with quoted handler should produce Calls(handleClick); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_on_event_attribute_curly_produces_calls() {
    // on:click="{handler}" with curly brace value → Calls(handler)
    let r = extract::extract(
        r#"<button on:click="{handleClick}">Click</button>"#,
        "Button.svelte",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "handleClick"),
        "on:click with curly handler should produce Calls(handleClick); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_on_submit_event_produces_calls() {
    // on:submit="onSubmit" on a form element → Calls(onSubmit)
    let r = extract::extract(
        r#"<form on:submit="onSubmit"><input /></form>"#,
        "Form.svelte",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "onSubmit"),
        "on:submit directive should produce Calls(onSubmit); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// {#each} / {#if} / {#await} block — leading identifier extraction
//
// TODO: The HTML grammar (tree-sitter-html) does not produce `raw_text` nodes
// for Svelte control-flow block syntax at template level. The
// `extract_svelte_blocks` scanner in extract.rs only fires on `raw_text`
// children, which appear inside <script>/<style> elements in the HTML grammar.
// Template-level `{#each}`, `{#if}`, `{#await}` blocks are tokenised as
// `text` nodes and are currently not traversed. These would require
// tree-sitter-svelte to handle correctly.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// standard HTML — no spurious Calls for native tags
// ---------------------------------------------------------------------------

#[test]
fn cov_lowercase_html_tags_do_not_produce_calls() {
    // Native HTML tags must not produce Calls edges
    let r = extract::extract(
        "<div><p><span>text</span></p></div>",
        "Layout.svelte",
    );
    let html_calls: Vec<_> = r.refs.iter()
        .filter(|rf| rf.kind == EdgeKind::Calls)
        .filter(|rf| matches!(rf.target_name.as_str(), "div" | "Div" | "p" | "P" | "span" | "Span"))
        .collect();
    assert!(
        html_calls.is_empty(),
        "standard HTML tags must not produce Calls; got: {:?}",
        html_calls.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}
