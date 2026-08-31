// =============================================================================
// engine/type_mention_demand_tests — pulls sourced from signature and chain
// type mentions
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::type_checker::core::types::TypeArena;
use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, ParsedFile, SegmentKind,
    SymbolKind,
};

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
    let tree = Compilation::build(&[], &Default::default(), arena);

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
    loc.insert(
        "microsoft.entityframeworkcore",
        "PropertyBuilder",
        ef_file.clone(),
    );

    // Tree already holds an UNRELATED class with the same bare name.
    let mut decoy_class = method_with_signature("");
    decoy_class.name = "PropertyBuilder".into();
    decoy_class.qualified_name = "System.Reflection.Emit.PropertyBuilder".into();
    decoy_class.kind = SymbolKind::Class;
    decoy_class.signature = None;
    let decoy = external_file("ext:dotnet:CoreLib/emit.cs", vec![decoy_class]);
    let mut id_map = HashMap::new();
    id_map.insert(
        (
            "ext:dotnet:CoreLib/emit.cs".to_string(),
            "System.Reflection.Emit.PropertyBuilder".to_string(),
        ),
        1i64,
    );
    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(std::slice::from_ref(&decoy), &id_map.clone().into(), arena);

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

/// A delegate-wrapped callback parameter in a cracked signature names the
/// type a caller's lambda parameter will be seeded as — that type must be
/// demanded even though no ref and no return head ever names it.
#[test]
fn callback_param_type_head_is_demanded() {
    let symbol = method_with_signature(
        "CreateTable(string, Action<Microsoft.EntityFrameworkCore.Migrations.Operations.Builders.ColumnsBuilder>): OperationBuilder",
    );

    let mut loc = SymbolLocationIndex::new();
    let builder_file = PathBuf::from(
        "ext:dotnet-type:/pkgs/efrel.dll!!efrel!!Microsoft.EntityFrameworkCore.Migrations.Operations.Builders.ColumnsBuilder",
    );
    loc.insert(
        "microsoft.entityframeworkcore.relational",
        "ColumnsBuilder",
        builder_file.clone(),
    );
    // The return head resolves elsewhere; only the callback arg matters here.
    loc.insert(
        "microsoft.entityframeworkcore.relational",
        "OperationBuilder",
        PathBuf::from("ext:x"),
    );

    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), arena);

    let profiles = crate::indexer::resolve::engine::pipeline::_test_build_profiles();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    super::collect_callback_param_type_files(
        std::slice::from_ref(&symbol),
        "csharp",
        &profiles,
        &tree,
        &loc,
        &mut seen,
        &mut out,
    );

    assert_eq!(
        out,
        vec![builder_file],
        "the Action<> type argument must be demand-pulled"
    );
}

/// A chain rooted on a keyword receiver carries its aliased library type on the
/// root segment (`string` → `System.String`). No ref names that type, so the
/// chain-root collector is the only thing that can materialize it.
#[test]
fn chain_root_declared_type_is_demanded() {
    let r = chain_ref("IsNullOrWhiteSpace", "string", Some("System.String"));

    let mut loc = SymbolLocationIndex::new();
    let string_file =
        PathBuf::from("ext:dotnet-type:/dotnet/System.Runtime.dll!!System.Runtime!!System.String");
    loc.insert("system.runtime", "String", string_file.clone());

    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), arena);

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    super::collect_chain_root_type_files(
        std::slice::from_ref(&r),
        &tree,
        &loc,
        &mut seen,
        &mut out,
    );

    assert_eq!(
        out,
        vec![string_file],
        "the root segment's declared type must be demand-pulled"
    );
}

/// Several modules offer the same bare leaf. Only the entry whose virtual path
/// ADDRESSES the qualified head is the declaration the root names — the others
/// are same-named types from unrelated assemblies and must not be pulled.
#[test]
fn qualified_chain_root_pulls_only_the_addressed_entry() {
    let r = chain_ref("IsNullOrWhiteSpace", "string", Some("System.String"));

    let mut loc = SymbolLocationIndex::new();
    let addressed =
        PathBuf::from("ext:dotnet-type:/dotnet/System.Runtime.dll!!System.Runtime!!System.String");
    loc.insert("system.runtime", "String", addressed.clone());
    loc.insert(
        "vendor.text",
        "String",
        PathBuf::from("ext:dotnet-type:/pkgs/vendor.dll!!vendor!!Vendor.Text.String"),
    );

    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), arena);

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    super::collect_chain_root_type_files(
        std::slice::from_ref(&r),
        &tree,
        &loc,
        &mut seen,
        &mut out,
    );

    assert_eq!(
        out,
        vec![addressed],
        "a qualified head picks its own declaration, not every module offering the leaf"
    );
}

/// A BARE declared head is high-cardinality evidence — every module declaring
/// the leaf offers an entry, none of which the root is known to name. The seed
/// pass demands nothing for it.
#[test]
fn bare_chain_root_declared_type_pulls_nothing() {
    let r = chain_ref("Unwrap", "value", Some("Result"));

    let mut loc = SymbolLocationIndex::new();
    loc.insert("crate.a", "Result", PathBuf::from("ext:a"));
    loc.insert("crate.b", "Result", PathBuf::from("ext:b"));

    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), arena);

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    super::collect_chain_root_type_files(
        std::slice::from_ref(&r),
        &tree,
        &loc,
        &mut seen,
        &mut out,
    );

    assert!(
        out.is_empty(),
        "a bare root head fans out over modules — declined"
    );
}

/// A root with no declared type names nothing to pull.
#[test]
fn chain_root_without_declared_type_pulls_nothing() {
    let r = chain_ref("ToString", "value", None);

    let mut loc = SymbolLocationIndex::new();
    loc.insert("system.runtime", "String", PathBuf::from("ext:string"));

    let arena = Arc::new(TypeArena::new());
    let tree = Compilation::build(&[], &Default::default(), arena);

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    super::collect_chain_root_type_files(
        std::slice::from_ref(&r),
        &tree,
        &loc,
        &mut seen,
        &mut out,
    );

    assert!(out.is_empty(), "an untyped root demands nothing");
}

fn external_file(path: &str, symbols: Vec<ExtractedSymbol>) -> ParsedFile {
    ParsedFile {
        path: path.into(),
        language: "csharp".into(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
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

/// A two-segment call chain `<root>.<target>()` whose root optionally carries a
/// declared type.
fn chain_ref(target: &str, root_name: &str, root_type: Option<&str>) -> ExtractedRef {
    let root = segment(root_name, root_type, false);
    let member = segment(target, None, true);
    ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.into(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: Some(MemberChain {
            segments: vec![root, member],
        }),
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn segment(name: &str, declared_type: Option<&str>, is_call: bool) -> ChainSegment {
    ChainSegment {
        name: name.into(),
        node_kind: "identifier".into(),
        kind: SegmentKind::Identifier,
        declared_type: declared_type.map(str::to_string),
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        type_arg_ids: Vec::new(),
        is_call,
        call_args: Vec::new(),
    }
}
