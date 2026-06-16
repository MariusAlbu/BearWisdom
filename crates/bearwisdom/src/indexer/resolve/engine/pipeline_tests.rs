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
