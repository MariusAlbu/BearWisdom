// =============================================================================
// engine/externals_demand_tests.rs — unit tests for demand-driven pulls
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::type_checker::core::types::TypeArena;
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind};

fn method_with_signature(sig: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: "make".into(),
        qualified_name: "Thing.make".into(),
        kind: SymbolKind::Method,
        visibility: None,
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: Some(sig.into()),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

/// A Rust method's return type is a `::`-qualified path (`gadgetcrate::Gadget`),
/// never emitted as a `.`-joined string the way a TS/namespace type is. The
/// return-type-head closure must (a) run for a non-TS/TSX language at all, and
/// (b) reduce the path to its bare leaf before looking it up in the location
/// index — the index keys locations by bare declared name, not by path.
#[test]
fn collect_return_type_files_follows_rust_path_qualified_head() {
    let symbol = method_with_signature("pub fn make(&self) -> gadgetcrate::Gadget");

    let mut loc = SymbolLocationIndex::new();
    let gadget_file = PathBuf::from("/fake/registry/gadgetcrate-0.1.0/src/lib.rs");
    loc.insert("gadgetcrate", "Gadget", gadget_file.clone());

    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &HashMap::new(), arena);

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    super::collect_return_type_files(
        std::slice::from_ref(&symbol),
        "rust",
        &tree,
        &loc,
        &mut seen,
        &mut out,
    );

    assert_eq!(
        out,
        vec![gadget_file],
        "a Rust return-type head must be followed for a non-TS language and its \
         `::`-qualified path reduced to the bare leaf the location index keys on"
    );
}

/// A qualified return-type head must be guarded by QUALIFIED-name presence,
/// not bare-leaf presence: a same-named type from an unrelated module already
/// in the tree (`System.Reflection.Emit.PropertyBuilder`) must not suppress
/// pulling the module the signature actually names
/// (`Microsoft.EntityFrameworkCore.Metadata.Builders.PropertyBuilder`).
#[test]
fn qualified_return_head_not_suppressed_by_same_named_type() {
    let symbol = method_with_signature(
        "Property(string): Microsoft.EntityFrameworkCore.Metadata.Builders.PropertyBuilder",
    );

    let mut loc = SymbolLocationIndex::new();
    let ef_file = PathBuf::from(
        "ext:dotnet-type:/pkgs/efcore/lib/ef.dll!!Microsoft.EntityFrameworkCore\
         !!Microsoft.EntityFrameworkCore.Metadata.Builders.PropertyBuilder",
    );
    loc.insert("microsoft.entityframeworkcore", "PropertyBuilder", ef_file.clone());

    // Tree already holds an UNRELATED class with the same bare name.
    let mut decoy_class = method_with_signature("");
    decoy_class.name = "PropertyBuilder".into();
    decoy_class.qualified_name = "System.Reflection.Emit.PropertyBuilder".into();
    decoy_class.kind = SymbolKind::Class;
    decoy_class.signature = None;
    let decoy = ParsedFile {
        path: "ext:dotnet:CoreLib/emit.cs".into(),
        language: "csharp".into(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: vec![decoy_class],
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
            "ext:dotnet:CoreLib/emit.cs".to_string(),
            "System.Reflection.Emit.PropertyBuilder".to_string(),
        ),
        1i64,
    );
    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(std::slice::from_ref(&decoy), &id_map, arena);

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    super::collect_return_type_files(
        std::slice::from_ref(&symbol),
        "csharp",
        &tree,
        &loc,
        &mut seen,
        &mut out,
    );

    assert_eq!(
        out,
        vec![ef_file],
        "the qualified head must be pulled even though a same-named type from \
         another module is already indexed"
    );
}

/// An EXTERNAL symbol already in the tree (eager stdlib pass, earlier closure
/// iteration) must not veto the demand pull for a same-named ref — the
/// internal-wins rule gates on INTERNAL definitions only. One package's
/// `Assert` method must not suppress pulling another package's `Assert` class.
#[test]
fn external_same_name_symbol_does_not_veto_ref_pull() {
    use crate::types::{EdgeKind, ExtractedRef};

    let mut squatter = method_with_signature("");
    squatter.name = "Assert".into();
    squatter.qualified_name = "System.Diagnostics.Debug.Assert".into();
    squatter.signature = None;
    let ext_pf = ParsedFile {
        path: "ext:dotnet:CoreLib/debug.cs".into(),
        language: "csharp".into(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: vec![squatter],
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
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    super::collect_external_files(std::slice::from_ref(&r), &tree, &loc, &mut seen, &mut out);
    assert_eq!(
        out,
        vec![assert_file],
        "an external same-name squatter must not suppress the pull"
    );
}
