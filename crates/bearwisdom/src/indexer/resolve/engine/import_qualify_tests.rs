// =============================================================================
// engine/import_qualify_tests.rs — unit tests for import-scoped head
// requalification
// =============================================================================

use std::collections::HashMap;
use std::sync::Arc;

use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::type_checker::core::types::{Type, TypeArena};
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility,
};

fn make_symbol(
    name: &str,
    qname: &str,
    kind: SymbolKind,
    parent_index: Option<usize>,
    declared_type: Option<crate::type_checker::core::types::TypeId>,
    return_type: Option<crate::type_checker::core::types::TypeId>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
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
        declared_type,
        return_type,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn import_ref(local: &str, module: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: local.to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        col: 0,
        module: Some(module.to_string()),
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
        is_import_binding: true,
        is_reexport: false,
    }
}

fn make_parsed_file(
    path: &str,
    symbols: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "typescript".to_string(),
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
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

/// `class Wrapper { getClient(): Client }` with `import { Client } from
/// "@ms/graph"`, compiled BEFORE the external package materializes.
fn internal_file(arena: &TypeArena) -> (ParsedFile, HashMap<(String, String), i64>) {
    let client_bare = arena.class("Client");
    let symbols = vec![
        make_symbol("Wrapper", "Wrapper", SymbolKind::Class, None, None, None),
        make_symbol(
            "getClient",
            "Wrapper.getClient",
            SymbolKind::Method,
            Some(0),
            None,
            Some(client_bare),
        ),
    ];
    let refs = vec![import_ref("Client", "@ms/graph")];
    let pf = make_parsed_file("src/wrapper.ts", symbols, refs);
    let mut id_map = HashMap::new();
    id_map.insert(("src/wrapper.ts".to_string(), "Wrapper".to_string()), 1);
    id_map.insert(("src/wrapper.ts".to_string(), "Wrapper.getClient".to_string()), 2);
    (pf, id_map)
}

fn external_file() -> (ParsedFile, HashMap<(String, String), i64>) {
    let symbols = vec![make_symbol(
        "Client",
        "@ms/graph.Client",
        SymbolKind::Class,
        None,
        None,
        None,
    )];
    let pf = make_parsed_file("ext:ts:@ms/graph/index.d.ts", symbols, Vec::new());
    let mut id_map = HashMap::new();
    id_map.insert((
        "ext:ts:@ms/graph/index.d.ts".to_string(),
        "@ms/graph.Client".to_string(),
    ), 100);
    (pf, id_map)
}

fn fmt(tree: &Compilation, id: i64) -> String {
    let arena = tree.type_arena().expect("arena");
    let ty = tree.return_type_id_of(id).expect("return type");
    arena.format_type(ty)
}

#[test]
fn bare_head_requalifies_when_external_materializes_in_later_ingest() {
    let arena = Arc::new(TypeArena::new());
    let (internal, id_map) = internal_file(&arena);
    let mut tree = Compilation::build(&[internal], &id_map, Arc::clone(&arena));

    // Candidate absent in the internal batch — the head must stay bare.
    assert_eq!(fmt(&tree, 2), "Client");

    let (external, ext_ids) = external_file();
    tree.ingest(&[external], &ext_ids, &std::collections::HashSet::new());

    assert_eq!(fmt(&tree, 2), "@ms/graph.Client");
}

#[test]
fn bare_head_requalifies_within_a_single_batch() {
    let arena = Arc::new(TypeArena::new());
    let (internal, mut id_map) = internal_file(&arena);
    let (external, ext_ids) = external_file();
    id_map.extend(ext_ids);
    let tree = Compilation::build(&[internal, external], &id_map, Arc::clone(&arena));

    assert_eq!(fmt(&tree, 2), "@ms/graph.Client");
}

#[test]
fn applied_argument_heads_requalify_alongside_the_base() {
    let arena = Arc::new(TypeArena::new());
    let base = arena.class("Promise");
    let arg = arena.class("Client");
    let applied = arena.intern(Type::Apply { base, args: vec![arg] });

    let symbols = vec![
        make_symbol("Wrapper", "Wrapper", SymbolKind::Class, None, None, None),
        make_symbol(
            "fetch",
            "Wrapper.fetch",
            SymbolKind::Method,
            Some(0),
            None,
            Some(applied),
        ),
    ];
    let refs = vec![import_ref("Client", "@ms/graph")];
    let pf = make_parsed_file("src/wrapper.ts", symbols, refs);
    let mut id_map = HashMap::new();
    id_map.insert(("src/wrapper.ts".to_string(), "Wrapper".to_string()), 1);
    id_map.insert(("src/wrapper.ts".to_string(), "Wrapper.fetch".to_string()), 2);
    let (external, ext_ids) = external_file();
    id_map.extend(ext_ids);
    let tree = Compilation::build(&[pf, external], &id_map, Arc::clone(&arena));

    assert_eq!(fmt(&tree, 2), "Promise<@ms/graph.Client>");
}

#[test]
fn subpath_specifier_falls_back_to_the_package_root_qname() {
    let arena = Arc::new(TypeArena::new());
    let client_bare = arena.class("Client");
    let symbols = vec![
        make_symbol("Wrapper", "Wrapper", SymbolKind::Class, None, None, None),
        make_symbol(
            "getClient",
            "Wrapper.getClient",
            SymbolKind::Method,
            Some(0),
            None,
            Some(client_bare),
        ),
    ];
    let refs = vec![import_ref("Client", "@ms/graph/lib/client")];
    let pf = make_parsed_file("src/wrapper.ts", symbols, refs);
    let mut id_map = HashMap::new();
    id_map.insert(("src/wrapper.ts".to_string(), "Wrapper".to_string()), 1);
    id_map.insert(("src/wrapper.ts".to_string(), "Wrapper.getClient".to_string()), 2);
    let (external, ext_ids) = external_file();
    id_map.extend(ext_ids);
    let tree = Compilation::build(&[pf, external], &id_map, Arc::clone(&arena));

    assert_eq!(fmt(&tree, 2), "@ms/graph.Client");
}

#[test]
fn locally_declared_type_shadows_the_import() {
    let arena = Arc::new(TypeArena::new());
    let client_bare = arena.class("Client");
    let symbols = vec![
        make_symbol("Wrapper", "Wrapper", SymbolKind::Class, None, None, None),
        make_symbol(
            "getClient",
            "Wrapper.getClient",
            SymbolKind::Method,
            Some(0),
            None,
            Some(client_bare),
        ),
        // A same-file `class Client` — the annotation names THIS one.
        make_symbol("Client", "Client", SymbolKind::Class, None, None, None),
    ];
    let refs = vec![import_ref("Client", "@ms/graph")];
    let pf = make_parsed_file("src/wrapper.ts", symbols, refs);
    let mut id_map = HashMap::new();
    id_map.insert(("src/wrapper.ts".to_string(), "Wrapper".to_string()), 1);
    id_map.insert(("src/wrapper.ts".to_string(), "Wrapper.getClient".to_string()), 2);
    id_map.insert(("src/wrapper.ts".to_string(), "Client".to_string()), 3);
    let (external, ext_ids) = external_file();
    id_map.extend(ext_ids);
    let tree = Compilation::build(&[pf, external], &id_map, Arc::clone(&arena));

    assert_eq!(fmt(&tree, 2), "Client");
}

#[test]
fn relative_import_never_requalifies() {
    let arena = Arc::new(TypeArena::new());
    let client_bare = arena.class("Client");
    let symbols = vec![
        make_symbol("Wrapper", "Wrapper", SymbolKind::Class, None, None, None),
        make_symbol(
            "getClient",
            "Wrapper.getClient",
            SymbolKind::Method,
            Some(0),
            None,
            Some(client_bare),
        ),
    ];
    let refs = vec![import_ref("Client", "./client")];
    let pf = make_parsed_file("src/wrapper.ts", symbols, refs);
    let mut id_map = HashMap::new();
    id_map.insert(("src/wrapper.ts".to_string(), "Wrapper".to_string()), 1);
    id_map.insert(("src/wrapper.ts".to_string(), "Wrapper.getClient".to_string()), 2);
    let (external, ext_ids) = external_file();
    id_map.extend(ext_ids);
    let tree = Compilation::build(&[pf, external], &id_map, Arc::clone(&arena));

    assert_eq!(fmt(&tree, 2), "Client");
}

#[test]
fn already_qualified_head_is_untouched() {
    let arena = Arc::new(TypeArena::new());
    let qualified = arena.class("other/pkg.Client");
    let symbols = vec![
        make_symbol("Wrapper", "Wrapper", SymbolKind::Class, None, None, None),
        make_symbol(
            "getClient",
            "Wrapper.getClient",
            SymbolKind::Method,
            Some(0),
            None,
            Some(qualified),
        ),
    ];
    let refs = vec![import_ref("Client", "@ms/graph")];
    let pf = make_parsed_file("src/wrapper.ts", symbols, refs);
    let mut id_map = HashMap::new();
    id_map.insert(("src/wrapper.ts".to_string(), "Wrapper".to_string()), 1);
    id_map.insert(("src/wrapper.ts".to_string(), "Wrapper.getClient".to_string()), 2);
    let (external, ext_ids) = external_file();
    id_map.extend(ext_ids);
    let tree = Compilation::build(&[pf, external], &id_map, Arc::clone(&arena));

    assert_eq!(fmt(&tree, 2), "other/pkg.Client");
}
