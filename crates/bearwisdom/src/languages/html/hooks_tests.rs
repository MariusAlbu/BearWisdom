// Sibling tests for `hooks.rs` — `<script src>` host-linking of inline-script
// calls to functions defined in referenced project JS files.

use super::HtmlHooks;
use crate::indexer::resolve::legacy::{build_scope_chain, RefContext, Resolution, SymbolIndex};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::*;
use std::collections::HashMap;

fn fn_symbol(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn host_class(name: &str) -> ExtractedSymbol {
    let mut s = fn_symbol(name);
    s.kind = SymbolKind::Class;
    s
}

/// A `<script src="url">` Imports ref as the HTML extractor emits it.
fn script_src_ref(url: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: url.to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        col: 0,
        module: Some(url.to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

/// A bare inline-`<script>` call ref (effective language js).
fn inline_call_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 5,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_html_file(path: &str, refs: Vec<ExtractedRef>) -> ParsedFile {
    make_file(path, "html", vec![host_class("index")], refs)
}

fn make_js_file(path: &str, fns: &[&str]) -> ParsedFile {
    make_file(
        path,
        "javascript",
        fns.iter().map(|n| fn_symbol(n)).collect(),
        vec![],
    )
}

fn make_file(
    path: &str,
    lang: &str,
    syms: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: lang.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: syms,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn build_env(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in files {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }
    let owned: Vec<ParsedFile> = files
        .iter()
        .map(|f| make_file(&f.path, &f.language, f.symbols.clone(), f.refs.clone()))
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

/// Drive an inline-script call ref through HTML's `resolve_ref` host-hook,
/// using a host context built from the HTML file's `<script src>` refs — the
/// same shape the resolve loop's cross-lang-embedded fallback supplies.
fn resolve(
    html: &ParsedFile,
    call: &ExtractedRef,
    index: &SymbolIndex,
) -> Option<Resolution> {
    let file_ctx = HtmlHooks.build_file_context(html, None).unwrap();
    let ref_ctx = RefContext {
        extracted_ref: call,
        source_symbol: &html.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    HtmlHooks.resolve_ref(&file_ctx, &ref_ctx, index)
}

#[test]
fn inline_call_binds_to_referenced_script_function() {
    // `<script src="./app.js">` + inline `doThing()`; `doThing` lives in app.js.
    let app = make_js_file("pages/app.js", &["doThing"]);
    let html = make_html_file("pages/index.html", vec![script_src_ref("./app.js")]);
    let (index, id_map) = build_env(&[&app, &html]);

    let call = inline_call_ref("doThing");
    let res = resolve(&html, &call, &index).expect("inline call should bind to app.js fn");
    assert_eq!(res.strategy, "html_script_src");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("pages/app.js".to_string(), "doThing".to_string()))
            .unwrap()
    );
}

#[test]
fn bare_src_without_dot_prefix_resolves() {
    // `<script src="app.js">` (no leading `./`) is still host-directory-relative.
    let app = make_js_file("pages/app.js", &["doThing"]);
    let html = make_html_file("pages/index.html", vec![script_src_ref("app.js")]);
    let (index, _id_map) = build_env(&[&app, &html]);
    let call = inline_call_ref("doThing");
    assert!(resolve(&html, &call, &index).is_some());
}

#[test]
fn cdn_src_produces_no_import_ref() {
    // Fixture (e): a CDN `<script src="https://…">` is filtered at extraction
    // time, so it never yields an Imports ref to host-link through.
    let src = r#"<html><body>
<script src="https://cdn.example.com/jquery.js"></script>
<script>doThing()</script>
</body></html>"#;
    let r = super::super::extract::extract(src, "pages/index.html");
    assert!(
        r.refs.iter().all(|rf| rf.kind != EdgeKind::Imports),
        "CDN script src must not produce an Imports ref, got {:?}",
        r.refs
    );
}

#[test]
fn cdn_src_attempts_no_binding() {
    // With no script-src import entry (CDN filtered upstream), a same-named
    // project symbol must not bind through a CDN link.
    let app = make_js_file("vendor/jquery.js", &["doThing"]);
    let html = make_html_file("pages/index.html", vec![]);
    let (index, _id_map) = build_env(&[&app, &html]);
    let call = inline_call_ref("doThing");
    assert!(
        resolve(&html, &call, &index).is_none(),
        "a CDN-linked page provides no script-src import, so no bind"
    );
}

#[test]
fn unlinked_function_does_not_bind() {
    // `doThing` exists in a project JS file the page never references — no
    // `<script src>` link names it, so it must stay unresolved.
    let other = make_js_file("pages/other.js", &["doThing"]);
    let html = make_html_file("pages/index.html", vec![script_src_ref("./app.js")]);
    let (index, _id_map) = build_env(&[&other, &html]);
    let call = inline_call_ref("doThing");
    assert!(
        resolve(&html, &call, &index).is_none(),
        "a function in an unreferenced file must not bind"
    );
}

#[test]
fn parent_relative_src_resolves() {
    // `<script src="../lib/util.js">` from `pages/sub/index.html` resolves to
    // `pages/lib/util.js`.
    let util = make_js_file("pages/lib/util.js", &["helper"]);
    let html = make_html_file("pages/sub/index.html", vec![script_src_ref("../lib/util.js")]);
    let (index, _id_map) = build_env(&[&util, &html]);
    let call = inline_call_ref("helper");
    let res = resolve(&html, &call, &index).expect("parent-relative src should resolve");
    assert_eq!(res.strategy, "html_script_src");
}

#[test]
fn ambiguous_target_across_links_declines() {
    // Two referenced scripts each define `init`; the inline call can't pick one.
    let a = make_js_file("pages/a.js", &["init"]);
    let b = make_js_file("pages/b.js", &["init"]);
    let html = make_html_file(
        "pages/index.html",
        vec![script_src_ref("./a.js"), script_src_ref("./b.js")],
    );
    let (index, _id_map) = build_env(&[&a, &b, &html]);
    let call = inline_call_ref("init");
    assert!(
        resolve(&html, &call, &index).is_none(),
        "two linked scripts defining the same function must decline"
    );
}
