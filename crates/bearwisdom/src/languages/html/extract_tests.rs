// Sibling test file for `extract.rs`.

use super::*;

#[test]
fn file_host_symbol_named_after_stem() {
    let r = extract("<html></html>", "docs/index.html");
    assert_eq!(r.symbols[0].name, "index");
    assert_eq!(r.symbols[0].kind, SymbolKind::Class);
}

#[test]
fn element_id_becomes_anchor_symbol() {
    let src = r#"<html><body><section id="intro">Hi</section><div id="footer"></div></body></html>"#;
    let r = extract(src, "page.html");
    let ids: Vec<&str> = r
        .symbols
        .iter()
        .skip(1)
        .map(|s| s.name.as_str())
        .collect();
    assert!(ids.contains(&"intro"));
    assert!(ids.contains(&"footer"));
}

#[test]
fn element_without_id_not_anchored() {
    let src = "<html><div>text</div></html>";
    let r = extract(src, "page.html");
    assert_eq!(r.symbols.len(), 1);
}

// ---------------------------------------------------------------------------
// Generator-marker detection (skip auto-generated HTML reports)
// ---------------------------------------------------------------------------

#[test]
fn generator_meta_skips_extraction() {
    // Robot Framework-style report file. Should produce zero symbols.
    let src = r#"<!DOCTYPE html>
<html>
<head>
<meta content="Robot Framework 3.2.2.dev1" name="Generator">
</head>
<body>
<div id="this-would-anchor">x</div>
</body>
</html>"#;
    let r = extract(src, "docs/Browser-1.0.0.html");
    assert!(r.symbols.is_empty(), "generated HTML must produce no symbols, got {:?}", r.symbols);
    assert!(r.refs.is_empty());
}

#[test]
fn html5_generator_meta_skipped() {
    let src = r#"<!DOCTYPE html>
<html><head><meta name="generator" content="JavaDoc 17"></head>
<body><div id="x"></div></body></html>"#;
    let r = extract(src, "doc/Class.html");
    assert!(r.symbols.is_empty());
}

#[test]
fn generator_meta_unquoted_attribute_skipped() {
    let src = r#"<html><head><meta name=Generator content="pydoc"></head>
<body><div id=hero></div></body></html>"#;
    let r = extract(src, "out.html");
    assert!(r.symbols.is_empty());
}

#[test]
fn ordinary_html_not_skipped() {
    let src = r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Hi</title></head>
<body><h1 id="hero">Hello</h1></body></html>"#;
    let r = extract(src, "page.html");
    assert!(!r.symbols.is_empty(), "ordinary HTML must still extract anchors");
    assert!(r.symbols.iter().any(|s| s.name == "hero"));
}

#[test]
fn generator_meta_outside_first_16kb_not_detected() {
    // If a generator marker only appears far past the head, we don't
    // bail. Real generators always put the marker in <head>, so this
    // is the right tradeoff for keeping the check O(1) per file.
    let mut src = String::from("<html><body>");
    src.push_str(&"<p>filler</p>".repeat(2000));
    src.push_str(r#"<meta name="generator" content="late marker">"#);
    src.push_str("</body></html>");
    let r = extract(&src, "page.html");
    assert!(!r.symbols.is_empty(), "marker past 16KB shouldn't bail extraction");
}
