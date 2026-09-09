use crate::types::{EdgeKind, ExtractedRef, ParsedFile};

fn blank_parsed_file(lang: &str) -> ParsedFile {
    ParsedFile {
        path: format!("f.{lang}"),
        language: lang.to_string(),
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
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

fn imports_ref(target_name: &str, module: &str, is_reexport: bool) -> ExtractedRef {
    ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport,
        source_symbol_index: 0,
        target_name: target_name.to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        col: 0,
        module: Some(module.to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

/// A dart `export 'foo.dart';` (`is_reexport=true`, `target_name="*"`) never
/// becomes a wildcard `ImportEntry` for the exporting file's own scope — an
/// export never opens the declaring file's own bare-name lookup, only what
/// OTHER files see when they import it.
#[test]
fn dart_export_star_ref_is_not_a_self_scope_wildcard() {
    let mut file = blank_parsed_file("dart");
    file.refs.push(imports_ref("*", "foo.dart", true));

    let ctx = super::build_file_context(
        "dart",
        &file,
        &crate::languages::dart::profile::DART_PROFILE,
        None,
        None,
    );

    let entry = ctx
        .imports
        .iter()
        .find(|i| i.imported_name == "*")
        .expect("the reexport ref still lands an entry");
    assert!(
        !entry.is_wildcard,
        "an is_reexport ref must never open the exporting file's own scope"
    );
}

/// A genuine dart wildcard IMPORT (`is_reexport=false`, `target_name="*"`)
/// keeps opening the importing file's own scope — the gate only excludes
/// re-export refs, not ordinary wildcard imports.
#[test]
fn dart_plain_wildcard_import_is_still_a_wildcard() {
    let mut file = blank_parsed_file("dart");
    file.refs.push(imports_ref("*", "flutter", false));

    let ctx = super::build_file_context(
        "dart",
        &file,
        &crate::languages::dart::profile::DART_PROFILE,
        None,
        None,
    );

    let entry = ctx
        .imports
        .iter()
        .find(|i| i.imported_name == "*")
        .expect("the wildcard import lands an entry");
    assert!(entry.is_wildcard, "a plain wildcard import stays a wildcard");
}

/// TS regression case: `export * from './x'` (the shape
/// `languages/typescript/reexports.rs` has emitted since before this gate
/// existed) must not retroactively open the barrel file's own scope either —
/// same gate, same generic closure, no TS-specific code involved.
#[test]
fn ts_export_star_ref_is_not_a_self_scope_wildcard() {
    let mut file = blank_parsed_file("typescript");
    file.refs.push(imports_ref("*", "./x", true));

    let ctx = super::build_file_context(
        "typescript",
        &file,
        &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
        None,
        None,
    );

    let entry = ctx
        .imports
        .iter()
        .find(|i| i.imported_name == "*")
        .expect("the reexport ref still lands an entry");
    assert!(
        !entry.is_wildcard,
        "a TS `export * from` ref must never open the barrel's own scope"
    );
}

/// A named re-export (`export { X } from './y'`, `is_reexport=true`,
/// `target_name="X"`) was never a wildcard candidate in the first place —
/// the gate's `!r.is_reexport` clause covers it the same way, but this locks
/// in that the non-wildcard shape is unaffected.
#[test]
fn named_reexport_ref_is_not_a_wildcard() {
    let mut file = blank_parsed_file("typescript");
    file.refs.push(imports_ref("X", "./y", true));

    let ctx = super::build_file_context(
        "typescript",
        &file,
        &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
        None,
        None,
    );

    let entry = ctx
        .imports
        .iter()
        .find(|i| i.imported_name == "X")
        .expect("the named reexport ref still lands an entry");
    assert!(!entry.is_wildcard);
}

#[test]
fn go_package_aliases_remain_bound_names_without_module_evidence() {
    let extracted = crate::languages::go::extract::extract(
        r#"package main

import (
    utilsstrings "example.com/utils/strings"
    clientpkg "example.com/api/client"
)
"#,
    );
    let mut file = blank_parsed_file("go");
    file.refs = extracted.refs;

    let ctx = super::build_file_context(
        "go",
        &file,
        &crate::languages::go::profile::GO_PROFILE,
        None,
        None,
    );

    assert!(ctx.imports.iter().any(|entry| {
        entry.imported_name == "utilsstrings" && entry.module_path.is_none() && !entry.is_wildcard
    }));
    assert!(ctx.imports.iter().any(|entry| {
        entry.imported_name == "clientpkg" && entry.module_path.is_none() && !entry.is_wildcard
    }));
}
