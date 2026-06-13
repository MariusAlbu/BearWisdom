// Sibling tests for `hooks.rs` — SCSS `@include`→`@mixin` / `@function`-call
// binding across same-file and `@use`/`@import` partial boundaries.

use super::super::SCSS_PROFILE;
use super::ScssHooks;
use crate::indexer::resolve::engine::{build_scope_chain, RefContext, Resolution, SymbolIndex};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::*;
use std::collections::HashMap;

fn mixin_symbol(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("@mixin {name}")),
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

fn class_symbol(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Class,
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

/// A `@include name(...)` Calls ref (no module).
fn include_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

/// A property-value `name(...)` call ref, tagged with the CSS-function hint the
/// extractor places on `call_expression`-derived calls.
fn css_fn_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: Some(super::super::extract::SCSS_CSS_FN_HINT.to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

/// A `@use 'path'` / `@import 'path'` Imports ref. `module` carries the raw
/// path; `build_file_context` derives the import entry from it.
fn import_ref(module: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: super::super::extract::path_to_target(module),
        kind: EdgeKind::Imports,
        line: 0,
        col: 0,
        module: Some(module.to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_file(path: &str, syms: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "scss".to_string(),
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
        .map(|f| make_file(&f.path, f.symbols.clone(), f.refs.clone()))
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

/// Drive `source.refs[ref_idx]` through the generic ladder, then the SCSS
/// `resolve_bare_post` hook — the same two-stage order the resolve loop uses.
fn resolve(source: &ParsedFile, ref_idx: usize, index: &SymbolIndex) -> Option<Resolution> {
    let file_ctx = ScssHooks.build_file_context(source, None).unwrap();
    let ref_ctx = RefContext {
        extracted_ref: &source.refs[ref_idx],
        source_symbol: &source.symbols[0],
        scope_chain: build_scope_chain(source.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };
    DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: index,
        kind_compatible: |_, _| true,
    }
    .resolve_all_with_profile(&SCSS_PROFILE)
    .or_else(|| ScssHooks.resolve_bare_post(&ref_ctx, &file_ctx, index))
}

fn target_id(id_map: &HashMap<(String, String), i64>, path: &str, qname: &str) -> i64 {
    *id_map
        .get(&(path.to_string(), qname.to_string()))
        .expect("symbol id present")
}

#[test]
fn same_file_include_binds_mixin() {
    // `@mixin breakpoint($s) {...}` + `@include breakpoint(md);` in one file.
    let source = make_file(
        "_layout.scss",
        vec![class_symbol("page"), mixin_symbol("breakpoint")],
        vec![include_ref("breakpoint")],
    );
    let (index, id_map) = build_env(&[&source]);
    let res = resolve(&source, 0, &index).expect("same-file include should bind");
    assert_eq!(res.target_symbol_id, target_id(&id_map, "_layout.scss", "breakpoint"));
}

#[test]
fn use_partial_include_binds_cross_file_mixin() {
    // Mixin defined in `_mixins.scss`; caller `@use 'mixins'` (underscore +
    // extension stripped on disk) then `@include breakpoint(md)`.
    let mixins = make_file("scss/_mixins.scss", vec![mixin_symbol("breakpoint")], vec![]);
    let caller = make_file(
        "scss/layout.scss",
        vec![class_symbol("page")],
        vec![import_ref("mixins"), include_ref("breakpoint")],
    );
    let (index, id_map) = build_env(&[&mixins, &caller]);
    // refs[0] = @use import, refs[1] = @include.
    let res = resolve(&caller, 1, &index).expect("@use partial include should bind");
    assert_eq!(res.strategy, "scss_partial_include");
    assert_eq!(
        res.target_symbol_id,
        target_id(&id_map, "scss/_mixins.scss", "breakpoint")
    );
}

#[test]
fn import_partial_include_binds_cross_file_mixin() {
    // Same as above but with the legacy `@import 'mixins'` spelling.
    let mixins = make_file("scss/_mixins.scss", vec![mixin_symbol("breakpoint")], vec![]);
    let caller = make_file(
        "scss/layout.scss",
        vec![class_symbol("page")],
        vec![import_ref("mixins"), include_ref("breakpoint")],
    );
    let (index, _id_map) = build_env(&[&mixins, &caller]);
    let res = resolve(&caller, 1, &index).expect("@import partial include should bind");
    assert_eq!(res.strategy, "scss_partial_include");
}

#[test]
fn function_call_binds_cross_partial_function() {
    // A project `@function to-rem($px)` in `_functions.scss`, called as
    // `to-rem(16px)` in a value — the call carries the CSS-function hint that
    // declines on the generic ladder, so the partial hook must catch it.
    let fns = make_file("scss/_functions.scss", vec![mixin_symbol("to-rem")], vec![]);
    let caller = make_file(
        "scss/type.scss",
        vec![class_symbol("body")],
        vec![import_ref("functions"), css_fn_ref("to-rem")],
    );
    let (index, id_map) = build_env(&[&fns, &caller]);
    let res = resolve(&caller, 1, &index).expect("project function call should bind");
    assert_eq!(res.strategy, "scss_partial_include");
    assert_eq!(
        res.target_symbol_id,
        target_id(&id_map, "scss/_functions.scss", "to-rem")
    );
}

#[test]
fn css_builtin_call_does_not_bind() {
    // `rgb(...)` has no project `@function`/`@mixin` symbol; the hint-tagged
    // call must stay unresolved (no coincidental bind, no hardcoded list).
    let caller = make_file(
        "scss/type.scss",
        vec![class_symbol("body")],
        vec![css_fn_ref("rgb")],
    );
    let (index, _id_map) = build_env(&[&caller]);
    assert!(
        resolve(&caller, 0, &index).is_none(),
        "a CSS built-in call with no project symbol must not bind"
    );
}

#[test]
fn unimported_partial_include_does_not_bind() {
    // The mixin exists in a sibling partial the caller never `@use`s/`@import`s.
    // Without the import evidence the include must stay unresolved rather than
    // bind by bare name across files.
    let mixins = make_file("scss/_mixins.scss", vec![mixin_symbol("breakpoint")], vec![]);
    let caller = make_file(
        "scss/layout.scss",
        vec![class_symbol("page")],
        vec![include_ref("breakpoint")],
    );
    let (index, _id_map) = build_env(&[&mixins, &caller]);
    assert!(
        resolve(&caller, 0, &index).is_none(),
        "an include with no matching import must not bind across files"
    );
}

#[test]
fn ambiguous_cross_partial_include_declines() {
    // Two imported partials each define `breakpoint`; the include can't be
    // disambiguated, so the hook declines rather than guess.
    let a = make_file("scss/_a.scss", vec![mixin_symbol("breakpoint")], vec![]);
    let b = make_file("scss/_b.scss", vec![mixin_symbol("breakpoint")], vec![]);
    let caller = make_file(
        "scss/layout.scss",
        vec![class_symbol("page")],
        vec![import_ref("a"), import_ref("b"), include_ref("breakpoint")],
    );
    let (index, _id_map) = build_env(&[&a, &b, &caller]);
    assert!(
        resolve(&caller, 2, &index).is_none(),
        "two imported partials defining the same mixin must decline"
    );
}

#[test]
fn property_value_ref_unaffected() {
    // Regression guard: a class/property rule with no include/function call
    // produces no spurious binding from the hook.
    let caller = make_file(
        "scss/buttons.scss",
        vec![class_symbol("btn")],
        vec![css_fn_ref("darken")],
    );
    let (index, _id_map) = build_env(&[&caller]);
    assert!(
        resolve(&caller, 0, &index).is_none(),
        "a property-value call with no project symbol stays unresolved"
    );
}
