//! Tests for `script_tag_deps::parse_script_tag_deps`.
//!
//! Builds a fake project tree on disk (per test) shaped like an ASP.NET
//! MVC layout (`src/WebApp/wwwroot/lib/jquery/jquery.js`), runs the stage,
//! and asserts that referenced vendor files are parsed with language
//! detection.

use std::fs;

use super::parse_script_tag_deps;
use crate::types::{EdgeKind, ExtractedRef, FlowMeta, ParsedFile};

fn empty_parsed(path: &str, language: &str) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: language.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

#[test]
fn tilde_prefixed_url_resolves_to_wwwroot() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src/WebApp/wwwroot/lib/jquery")).unwrap();
    fs::create_dir_all(root.join("src/WebApp/Views/Shared")).unwrap();
    fs::write(
        root.join("src/WebApp/wwwroot/lib/jquery/jquery.js"),
        "var x = 1;\n",
    )
    .unwrap();
    fs::write(
        root.join("src/WebApp/Views/Shared/_Layout.cshtml"),
        "<script src=\"~/lib/jquery/jquery.js\"></script>",
    )
    .unwrap();

    let mut host = empty_parsed("src/WebApp/Views/Shared/_Layout.cshtml", "razor");
    host.refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: "~/lib/jquery/jquery.js".to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        module: Some("~/lib/jquery/jquery.js".to_string()),
        chain: None,
        byte_offset: 0,
    });

    let registry = crate::languages::default_registry();
    let out = parse_script_tag_deps(root, &[host], registry);
    assert_eq!(out.len(), 1, "expected jquery.js to be pulled in");
    assert_eq!(out[0].language, "javascript");
    assert!(out[0].path.ends_with("wwwroot/lib/jquery/jquery.js"));
}

#[test]
fn cdn_and_absolute_urls_filtered_at_extraction() {
    // Safety net: CDN refs shouldn't reach this stage (`extract_script_refs`
    // filters them), and resolution of the ones shaped like site-roots
    // falls through when no webroot exists.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("Views")).unwrap();
    fs::write(root.join("Views/Home.cshtml"), "<p/>").unwrap();

    let mut host = empty_parsed("Views/Home.cshtml", "razor");
    for url in &[
        "https://cdn.example.com/jquery.js",
        "http://localhost/foo.js",
        "//cdn.example.com/vue.js",
    ] {
        host.refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: url.to_string(),
            kind: EdgeKind::Imports,
            line: 0,
            module: Some(url.to_string()),
            chain: None,
            byte_offset: 0,
        });
    }

    let registry = crate::languages::default_registry();
    let out = parse_script_tag_deps(root, &[host], registry);
    assert!(out.is_empty());
}

#[test]
fn relative_url_resolves_against_host_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(root.join("pages/app.js"), "export const x = 1;\n").unwrap();
    fs::write(
        root.join("pages/index.html"),
        "<script src=\"app.js\"></script>",
    )
    .unwrap();

    let mut host = empty_parsed("pages/index.html", "html");
    host.refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: "app.js".to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        module: Some("app.js".to_string()),
        chain: None,
        byte_offset: 0,
    });

    let registry = crate::languages::default_registry();
    let out = parse_script_tag_deps(root, &[host], registry);
    assert_eq!(out.len(), 1);
    assert!(out[0].path.ends_with("pages/app.js"));
}

#[test]
fn already_parsed_file_not_duplicated() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("js")).unwrap();
    fs::write(root.join("js/app.js"), "const x = 1;").unwrap();
    fs::write(
        root.join("index.html"),
        "<script src=\"js/app.js\"></script>",
    )
    .unwrap();

    let host = {
        let mut pf = empty_parsed("index.html", "html");
        pf.refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: "js/app.js".to_string(),
            kind: EdgeKind::Imports,
            line: 0,
            module: Some("js/app.js".to_string()),
            chain: None,
            byte_offset: 0,
        });
        pf
    };
    let already_parsed = empty_parsed("js/app.js", "javascript");

    let registry = crate::languages::default_registry();
    let out = parse_script_tag_deps(root, &[host, already_parsed], registry);
    assert!(out.is_empty(), "already-parsed file must not be re-parsed");
}
