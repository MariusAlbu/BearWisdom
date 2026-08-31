use super::*;
use crate::types::{ExtractedRef, ExtractedSymbol, Visibility};

fn parsed_file(
    path: &str,
    content: Option<&str>,
    symbols: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "pascal".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: content.map(str::to_string),
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

fn namespace_symbol(name: &str, parent_index: Option<usize>) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn uses_ref(source_symbol_index: usize, unit_name: &str) -> ExtractedRef {
    ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index,
        target_name: unit_name.to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        col: 0,
        module: Some(unit_name.to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

/// A main unit file's shape, as `extract_unit` + `extract_uses` produce it:
/// symbol 0 is the unit's own Namespace, symbol 1 is the `"uses"` Namespace
/// `extract_uses` anchors its `Imports` refs to.
fn main_unit_file(path: &str, unit_name: &str, uses: &[&str]) -> ParsedFile {
    let symbols = vec![
        namespace_symbol(unit_name, None),
        namespace_symbol("uses", Some(0)),
    ];
    let refs = uses.iter().map(|u| uses_ref(1, u)).collect();
    parsed_file(path, None, symbols, refs)
}

fn fragment_file(path: &str, main_unit_directive: &str) -> ParsedFile {
    let content = format!("{{%MainUnit {main_unit_directive}}}\nbegin end.");
    parsed_file(path, Some(&content), Vec::new(), Vec::new())
}

fn wildcards_of<'a>(state: &'a PascalProjectState, path: &str) -> Vec<&'a str> {
    state.wildcards_for(path).iter().map(String::as_str).collect()
}

// ---------------------------------------------------------------------------
// parse_main_unit_directive
// ---------------------------------------------------------------------------

#[test]
fn parses_same_directory_main_unit_directive() {
    let src = "{%MainUnit castlesoundengine.pas}\n{ license header }";
    assert_eq!(
        parse_main_unit_directive(src),
        Some("castlesoundengine.pas".to_string())
    );
}

#[test]
fn parses_relative_main_unit_directive() {
    let src = "{%MainUnit ../castleutils.pas}\n";
    assert_eq!(
        parse_main_unit_directive(src),
        Some("../castleutils.pas".to_string())
    );
}

#[test]
fn ignores_dollar_directives_and_absent_main_unit() {
    let src = "{$ifdef FPC}\n{$I other.inc}\nbegin end.";
    assert_eq!(parse_main_unit_directive(src), None);
}

// ---------------------------------------------------------------------------
// resolve_relative
// ---------------------------------------------------------------------------

#[test]
fn resolve_relative_same_directory() {
    assert_eq!(
        resolve_relative(
            "src/audio/castlesoundengine_allocator.inc",
            "castlesoundengine.pas"
        ),
        Some("src/audio/castlesoundengine.pas".to_string())
    );
}

#[test]
fn resolve_relative_parent_directory() {
    assert_eq!(
        resolve_relative(
            "src/window/dialogs/castledialogviews_stuff.inc",
            "../castlewindow.pas"
        ),
        Some("src/window/castlewindow.pas".to_string())
    );
}

#[test]
fn resolve_relative_above_root_declines() {
    assert_eq!(
        resolve_relative("fragment.inc", "../../outside.pas"),
        None
    );
}

// ---------------------------------------------------------------------------
// build_main_unit_state
// ---------------------------------------------------------------------------

#[test]
fn fragment_inherits_main_units_own_name_and_uses_clause() {
    let main = main_unit_file(
        "src/audio/castlesoundengine.pas",
        "castlesoundengine",
        &["SysUtils", "Classes"],
    );
    let fragment = fragment_file(
        "src/audio/castlesoundengine_allocator.inc",
        "castlesoundengine.pas",
    );
    let state = build_main_unit_state(&[main, fragment], std::path::Path::new(""));

    let wildcards = wildcards_of(&state, "src/audio/castlesoundengine_allocator.inc");
    assert_eq!(wildcards, vec!["castlesoundengine", "SysUtils", "Classes"]);
}

#[test]
fn fragment_resolves_main_unit_in_parent_directory() {
    let main = main_unit_file("src/window/castlewindow.pas", "castlewindow", &["Classes"]);
    let fragment = fragment_file(
        "src/window/dialogs/castlewindow_dialogs.inc",
        "../castlewindow.pas",
    );
    let state = build_main_unit_state(&[main, fragment], std::path::Path::new(""));

    let wildcards = wildcards_of(&state, "src/window/dialogs/castlewindow_dialogs.inc");
    assert_eq!(wildcards, vec!["castlewindow", "Classes"]);
}

#[test]
fn fragment_without_directive_gets_no_wildcards() {
    let fragment = parsed_file(
        "src/audio/plain.inc",
        Some("begin end."),
        Vec::new(),
        Vec::new(),
    );
    let state = build_main_unit_state(&[fragment], std::path::Path::new(""));

    assert!(state.wildcards_for("src/audio/plain.inc").is_empty());
}

#[test]
fn include_directive_imports_are_excluded_from_uses_clause_units() {
    // The main unit's `Imports` ref for its own `{$I fragment.inc}` splice is
    // anchored to the unit's root symbol (index 0), not the `"uses"` symbol —
    // it must not leak into the fragment's wildcard list as a fake unit name.
    let mut main = main_unit_file("src/audio/castlesoundengine.pas", "castlesoundengine", &["SysUtils"]);
    main.refs.push(ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: "castlesoundengine_allocator".to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        col: 0,
        module: Some("castlesoundengine_allocator".to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
    let fragment = fragment_file(
        "src/audio/castlesoundengine_allocator.inc",
        "castlesoundengine.pas",
    );
    let state = build_main_unit_state(&[main, fragment], std::path::Path::new(""));

    let wildcards = wildcards_of(&state, "src/audio/castlesoundengine_allocator.inc");
    assert_eq!(wildcards, vec!["castlesoundengine", "SysUtils"]);
}

#[test]
fn unresolvable_main_unit_still_yields_own_name_wildcard() {
    // The `{%MainUnit}` target isn't present in `parsed` at all (e.g. a
    // partial reindex batch) — the fragment still gets its own declared main
    // unit's bare name as a wildcard, just without the `uses` clause.
    let fragment = fragment_file("src/audio/orphan.inc", "missing_unit.pas");
    let state = build_main_unit_state(&[fragment], std::path::Path::new(""));

    assert_eq!(
        wildcards_of(&state, "src/audio/orphan.inc"),
        vec!["missing_unit"]
    );
}
