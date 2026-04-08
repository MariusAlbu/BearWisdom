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

// ---------------------------------------------------------------------------
// client:* hydration directives — component still produces exactly one Calls
// ---------------------------------------------------------------------------

#[test]
fn cov_client_load_directive_does_not_duplicate_calls() {
    // client:load on a PascalCase component → still exactly one Calls(Counter),
    // not a second edge for the directive attribute itself
    let r = extract::extract("<Counter client:load />", "Page.astro");
    let counter_calls: Vec<_> = r.refs.iter()
        .filter(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "Counter")
        .collect();
    assert_eq!(
        counter_calls.len(),
        1,
        "client:load component should produce exactly one Calls(Counter); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_client_idle_directive_component_produces_calls() {
    // client:idle variant — same behaviour
    let r = extract::extract("<HeavyWidget client:idle></HeavyWidget>", "Page.astro");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "HeavyWidget"),
        "client:idle component should produce Calls(HeavyWidget); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// built-in Astro tags — Fragment and slot must not produce Calls edges
// ---------------------------------------------------------------------------

#[test]
fn cov_fragment_builtin_does_not_produce_calls() {
    // <Fragment> is an Astro built-in; must NOT produce a Calls edge
    let r = extract::extract("<Fragment><p>content</p></Fragment>", "Page.astro");
    assert!(
        !r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "Fragment"),
        "Fragment built-in must not produce Calls; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_slot_builtin_does_not_produce_calls() {
    // <slot /> is an Astro template injection point; must NOT produce Calls
    let r = extract::extract("<slot />", "Layout.astro");
    assert!(
        !r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "slot"),
        "slot built-in must not produce Calls; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// standard HTML — no spurious Calls edges for native tags
// ---------------------------------------------------------------------------

#[test]
fn cov_standard_html_elements_do_not_produce_calls() {
    // Lowercase HTML elements must never produce Calls
    let r = extract::extract(
        "<main><article><h1>Title</h1><p>Body</p></article></main>",
        "Post.astro",
    );
    let html_calls: Vec<_> = r.refs.iter()
        .filter(|rf| rf.kind == EdgeKind::Calls)
        .filter(|rf| matches!(
            rf.target_name.as_str(),
            "main" | "Main" | "article" | "Article" | "h1" | "H1" | "p" | "P"
        ))
        .collect();
    assert!(
        html_calls.is_empty(),
        "standard HTML elements must not produce Calls; got: {:?}",
        html_calls.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// nested PascalCase component inside native elements → still produces Calls
// ---------------------------------------------------------------------------

#[test]
fn cov_nested_component_inside_html_produces_calls() {
    // PascalCase component nested inside plain HTML wrapper → Calls edge
    let r = extract::extract(
        "<main><div><BlogCard /></div></main>",
        "Index.astro",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "BlogCard"),
        "nested PascalCase component should produce Calls(BlogCard); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// component Class symbol — correct stem from various filename patterns
// ---------------------------------------------------------------------------

#[test]
fn cov_component_symbol_simple_name() {
    // index.astro → Class(index) — filename preserved as-is (no PascalCase transform)
    let r = extract::extract("<div></div>", "index.astro");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "index"),
        "index.astro should produce Class(index); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}
