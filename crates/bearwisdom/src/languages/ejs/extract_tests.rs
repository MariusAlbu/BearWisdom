use super::extract as ejs_extract;
use crate::types::{EdgeKind, SymbolKind};

fn extract(src: &str, path: &str) -> crate::types::ExtractionResult {
    ejs_extract(src, path)
}

#[test]
fn file_emits_stem_class_symbol() {
    let r = extract("<p>hi</p>", "/views/page.ejs");
    let class = r.symbols.iter().find(|s| s.kind == SymbolKind::Class).unwrap();
    assert_eq!(class.name, "page");
}

#[test]
fn include_with_relative_path_emits_imports_ref() {
    let src = "<%- include('./partials/header') %>";
    let r = extract(src, "/views/index.ejs");
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Imports)
        .expect("expected an Imports ref for include()");
    assert_eq!(imp.target_name, "./partials/header");
}

#[test]
fn include_with_double_quotes_works() {
    let src = "<%- include(\"layout\") %>";
    let r = extract(src, "/views/index.ejs");
    let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports).unwrap();
    assert_eq!(imp.target_name, "layout");
}

#[test]
fn multiple_includes_each_emit_a_ref() {
    let src = "<%- include('./partials/header') %>\n<p>x</p>\n<%- include('./partials/footer') %>";
    let r = extract(src, "/views/index.ejs");
    let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
    assert_eq!(imports.len(), 2);
    let names: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"./partials/header"));
    assert!(names.contains(&"./partials/footer"));
}

#[test]
fn include_with_locals_object_only_captures_path() {
    let src = "<%- include('./partials/card', { user: u }) %>";
    let r = extract(src, "/views/index.ejs");
    let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports).unwrap();
    assert_eq!(imp.target_name, "./partials/card");
}

#[test]
fn identifier_suffixed_include_is_ignored() {
    // `xinclude(...)` is not the runtime include — must not be captured.
    let src = "<% xinclude('not-a-partial') %>";
    let r = extract(src, "/views/x.ejs");
    let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
    assert!(imports.is_empty(), "got: {:?}", imports);
}

#[test]
fn include_outside_ejs_tag_still_captured() {
    // EJS include can appear in raw `<% include('foo') %>` (1.x form) too.
    let src = "<% include('legacy') %>";
    let r = extract(src, "/views/x.ejs");
    let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports).unwrap();
    assert_eq!(imp.target_name, "legacy");
}
