// =============================================================================
// engine/externals_demand_tests — unit tests for demand-driven pulls
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::engine::demand_veto::{DemandVeto, FileLanguages};
use crate::type_checker::core::types::TypeArena;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind};

fn class_symbol(name: &str, qname: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.into(),
        qualified_name: qname.into(),
        kind: SymbolKind::Class,
        visibility: None,
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

/// An EXTERNAL symbol already in the tree (eager stdlib pass, earlier closure
/// iteration) must not veto the demand pull for a same-named ref — the
/// internal-wins rule gates on INTERNAL definitions only. One package's
/// `Assert` method must not suppress pulling another package's `Assert` class.
#[test]
fn external_same_name_symbol_does_not_veto_ref_pull() {
    let ext_pf = ParsedFile {
        path: "ext:dotnet:CoreLib/debug.cs".into(),
        language: "csharp".into(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: vec![class_symbol("Assert", "System.Diagnostics.Debug.Assert")],
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
    };
    let mut id_map = HashMap::new();
    id_map.insert(
        (
            "ext:dotnet:CoreLib/debug.cs".to_string(),
            "System.Diagnostics.Debug.Assert".to_string(),
        ),
        1i64,
    );
    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(std::slice::from_ref(&ext_pf), &id_map, arena);

    let mut loc = SymbolLocationIndex::new();
    let assert_file = PathBuf::from("ext:dotnet-type:/pkgs/xa.dll!!xunit.v3.assert!!Xunit.Assert");
    loc.insert("xunit.v3.assert", "Assert", assert_file.clone());

    let r = ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: "Assert".into(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };
    let profiles = crate::indexer::resolve::engine::pipeline::_test_build_profiles();
    let file_langs = FileLanguages::default();
    let veto = DemandVeto::new("csharp", &profiles, &file_langs);
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    super::collect_external_files(
        std::slice::from_ref(&r),
        &veto,
        &tree,
        &loc,
        &mut seen,
        &mut out,
    );
    assert_eq!(
        out,
        vec![assert_file],
        "an external same-name squatter must not suppress the pull"
    );
}

/// A `.pp` file pulled from a Pascal-hinted dep root must parse as Pascal
/// even though the registry's extension table maps `.pp` to Puppet
/// (`puppet`'s plugin registers first — see `languages/registry_init.rs`).
/// The hint set by `SymbolLocationIndex::tag_language` must win.
#[test]
fn parse_external_file_uses_language_hint_over_extension() {
    let dir = std::env::temp_dir().join(format!("bw_pp_hint_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("sysutils.pp");
    std::fs::write(&file, "unit SysUtils;\ninterface\nimplementation\nend.\n").unwrap();

    let mut loc = SymbolLocationIndex::new();
    loc.insert("fpc-rtl-objpas", "SysUtils", file.clone());
    loc.tag_language("pascal");

    let arena = Arc::new(TypeArena::new());
    let pf = super::parse_external_file(&file, &arena, &loc)
        .expect("a Pascal-hinted .pp file must parse");
    assert_eq!(pf.language, "pascal", "the hint must override the puppet extension mapping");

    std::fs::remove_dir_all(&dir).ok();
}

/// A `.pp` file with no recorded hint (no ecosystem tagged it) falls back to
/// the registry's extension table, unchanged from before this fix.
#[test]
fn parse_external_file_falls_back_to_extension_without_hint() {
    let dir = std::env::temp_dir().join(format!("bw_pp_no_hint_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("site.pp");
    std::fs::write(&file, "class site {}\n").unwrap();

    let loc = SymbolLocationIndex::new();
    let arena = Arc::new(TypeArena::new());
    let pf = super::parse_external_file(&file, &arena, &loc)
        .expect("an unhinted .pp file must still parse via the extension fallback");
    assert_eq!(
        pf.language, "puppet",
        "no hint recorded — the puppet plugin owns .pp in the extension table"
    );

    std::fs::remove_dir_all(&dir).ok();
}
