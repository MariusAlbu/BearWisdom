// =============================================================================
// astro/coverage_tests.rs
//
// Node-kind coverage for AstroPlugin::symbol_node_kinds() and ref_node_kinds().
// symbol_node_kinds: element, self_closing_element
// ref_node_kinds:    element, self_closing_element, attribute
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds: element / self_closing_element
// These produce the sentinel Class symbol inferred from the filename.
// ---------------------------------------------------------------------------

#[test]
fn cov_element_produces_component_class_symbol() {
    // The filename stem is used as the component Class symbol.
    let r = extract::extract("<div><p>Hello</p></div>", "BlogPost.astro");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "BlogPost"),
        "Astro page should produce Class symbol from filename; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_self_closing_element_pascal_produces_calls() {
    // PascalCase self-closing tag → Calls (ref_node_kinds: self_closing_element)
    let r = extract::extract("<UserCard />", "Index.astro");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "UserCard"),
        "PascalCase self-closing element should produce Calls(UserCard); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds: element (PascalCase) and attribute (client: hydration)
// ---------------------------------------------------------------------------

#[test]
fn cov_element_pascal_produces_calls() {
    // PascalCase open/close element → Calls edge (ref_node_kinds: element)
    let r = extract::extract("<Counter client:load></Counter>", "Page.astro");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "Counter"),
        "PascalCase element should produce Calls(Counter); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_kebab_element_produces_calls() {
    // Kebab-case element → normalised to PascalCase Calls
    let r = extract::extract("<my-widget></my-widget>", "Page.astro");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "MyWidget"),
        "kebab element should produce Calls(MyWidget); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
