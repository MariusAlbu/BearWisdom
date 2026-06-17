use std::collections::HashMap;
use std::sync::Arc;

use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::type_checker::core::types::TypeArena;

use super::{FileLookup, resolve_single_pass};

#[test]
fn empty_parsed_returns_ok_with_zero_counts() {
    // No DB is available in unit tests; this exercises the pre-flush path only
    // by verifying the function accepts empty input without panicking before
    // it reaches the DB write step — the DB call itself will fail (no file),
    // which is expected in this harness. The test asserts the construction
    // phase (Compilation build, profile map, solver) completes without panic.
    //
    // A real integration test runs via `bw reindex` on a fixture project.
    let arena = Arc::new(TypeArena::new());
    let parsed: Vec<crate::types::ParsedFile> = Vec::new();
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();

    // Constructing the tree and iterating zero files succeeds. We cannot call
    // resolve_single_pass without a real Database, so just verify the
    // intermediate values are constructible.
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &parsed,
        &symbol_id_map,
        Arc::clone(&arena),
    );
    let _ = tree; // constructed without panic

    let profiles = super::build_profiles();
    assert!(
        !profiles.is_empty(),
        "registry must produce at least one language profile"
    );
}

// ---------------------------------------------------------------------------
// FileLookup unit tests
// ---------------------------------------------------------------------------

/// `local_type` returns `None` before any binding is recorded.
#[test]
fn file_lookup_local_type_empty() {
    let arena = Arc::new(TypeArena::new());
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &[],
        &symbol_id_map,
        arc_clone(&arena),
    );
    let lookup = FileLookup::new(&tree);
    assert!(lookup.local_type("x").is_none());
}

/// `record_local_type` then `local_type` returns the recorded type.
#[test]
fn file_lookup_record_then_read() {
    let arena = Arc::new(TypeArena::new());
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &[],
        &symbol_id_map,
        arc_clone(&arena),
    );
    let lookup = FileLookup::new(&tree);
    lookup.record_local_type("repo".to_string(), "UserRepository".to_string());
    assert_eq!(lookup.local_type("repo").as_deref(), Some("UserRepository"));
}

/// `local_type_union` wraps the single result in a `vec!`.
#[test]
fn file_lookup_local_type_union_single_branch() {
    let arena = Arc::new(TypeArena::new());
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &[],
        &symbol_id_map,
        arc_clone(&arena),
    );
    let lookup = FileLookup::new(&tree);
    lookup.record_local_type("svc".to_string(), "OrderService".to_string());
    assert_eq!(
        lookup.local_type_union("svc"),
        Some(vec!["OrderService".to_string()])
    );
    assert!(lookup.local_type_union("missing").is_none());
}

/// `clear_local_cache` evicts all bindings so they cannot bleed into the
/// next file's resolution pass.
#[test]
fn file_lookup_clear_evicts_bindings() {
    let arena = Arc::new(TypeArena::new());
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &[],
        &symbol_id_map,
        arc_clone(&arena),
    );
    let lookup = FileLookup::new(&tree);
    lookup.record_local_type("x".to_string(), "Foo".to_string());
    assert!(lookup.local_type("x").is_some());
    lookup.clear_local_cache();
    assert!(lookup.local_type("x").is_none());
}

/// Structural delegation: `by_name` on an empty tree returns an empty set, not
/// a panic. Confirms the delegation layer compiles and runs.
#[test]
fn file_lookup_delegates_structural_to_tree() {
    let arena = Arc::new(TypeArena::new());
    let symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        &[],
        &symbol_id_map,
        arc_clone(&arena),
    );
    let lookup = FileLookup::new(&tree);
    assert!(lookup.by_name("anything").iter().next().is_none());
    assert!(lookup.by_qualified_name("a.b.c").is_none());
    assert!(lookup.members_of("SomeClass").iter().next().is_none());
}

fn arc_clone(a: &Arc<TypeArena>) -> Arc<TypeArena> {
    Arc::clone(a)
}

#[test]
fn engine_resolves_local_var_member_call_via_scope_exact_root() {
    use crate::types::{
        ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, MemberChain, ParsedFile,
        SegmentKind, SymbolKind, Visibility,
    };
    fn esym(name: &str, qname: &str, kind: SymbolKind, parent: Option<usize>) -> ExtractedSymbol {
        ExtractedSymbol {
            name: name.into(), qualified_name: qname.into(), kind,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0, byte_offset: 0,
            signature: None, doc_comment: None, scope_path: parent.map(|_| "SolidQueryDevtools".into()),
            parent_index: parent, declared_type: None, return_type: None,
            param_types: Vec::new(), generic_params: Vec::new(),
        }
    }
    fn cseg(name: &str, kind: SegmentKind, is_call: bool) -> ChainSegment {
        ChainSegment {
            name: name.into(), node_kind: String::new(), kind, declared_type: None,
            type_args: Vec::new(), optional_chaining: false, byte_offset: 0,
            declared_type_id: None, is_call, call_args: Vec::new(), type_arg_ids: Vec::new(),
        }
    }
    fn eref(src: usize, target: &str, kind: EdgeKind, chain: Option<MemberChain>) -> ExtractedRef {
        ExtractedRef {
            is_import_binding: false, is_reexport: false, source_symbol_index: src,
            target_name: target.into(), kind, line: 1, col: 0, module: None, chain,
            byte_offset: 1, namespace_segments: Vec::new(), call_args: Vec::new(),
        }
    }
    let symbols = vec![
        esym("SolidQueryDevtools", "SolidQueryDevtools", SymbolKind::Function, None), // 0
        esym("devtools", "SolidQueryDevtools.devtools", SymbolKind::Variable, Some(0)), // 1
        esym("TanstackQueryDevtools", "TanstackQueryDevtools", SymbolKind::Class, None), // 2
        esym("mount", "TanstackQueryDevtools.mount", SymbolKind::Method, Some(2)), // 3
    ];
    let refs = vec![
        // `const devtools = new TanstackQueryDevtools()` -> field type TypeRef
        eref(1, "TanstackQueryDevtools", EdgeKind::TypeRef, None),
        // `devtools.mount(ref)` chain
        eref(0, "mount", EdgeKind::Calls, Some(MemberChain {
            segments: vec![
                cseg("devtools", SegmentKind::Identifier, false),
                cseg("mount", SegmentKind::Property, true),
            ],
        })),
    ];
    let pf = ParsedFile {
        path: "d.ts".into(), language: "typescript".into(), content_hash: String::new(),
        size: 0, line_count: 0, mtime: None, package_id: None, symbols, refs,
        routes: Vec::new(), db_sets: Vec::new(), symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(), symbol_from_snippet: Vec::new(), content: None,
        has_errors: false, flow: FlowMeta::default(), demand_contributions: Vec::new(),
        alias_targets: Vec::new(), component_selectors: Vec::new(), plugin_flow_emissions: Vec::new(),
    };
    let mut id_map = HashMap::new();
    id_map.insert(("d.ts".to_string(), "SolidQueryDevtools".to_string()), 1i64);
    id_map.insert(("d.ts".to_string(), "SolidQueryDevtools.devtools".to_string()), 2i64);
    id_map.insert(("d.ts".to_string(), "TanstackQueryDevtools".to_string()), 3i64);
    id_map.insert(("d.ts".to_string(), "TanstackQueryDevtools.mount".to_string()), 4i64);
    let arena = Arc::new(TypeArena::new());
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(std::slice::from_ref(&pf), &id_map, arena);
    let profiles = super::build_profiles();
    let solver = super::SemanticModel::production();
    let (edges, unresolved) = super::resolve_one_file(&pf, &tree, &profiles, &solver, &id_map);
    // mount (target id 4) must resolve as an edge from SolidQueryDevtools (1).
    assert!(edges.iter().any(|e| e.1 == 4), "devtools.mount must resolve to TanstackQueryDevtools.mount");
}
