// =============================================================================
// indexer/expand_tests.rs — unit tests for indexer/expand.rs
//
// Kept in a sibling file so the production module stays free of synthetic
// fixture literals (dep names, file paths) that look like hardcoded
// production values to a casual reader.
// =============================================================================

use super::*;

#[test]
fn empty_misses_returns_zero_stats() {
    // Smoke test: we don't even touch the DB if there's nothing to do.
    // This is the hot path on a project with perfect resolution.
    let stats = ExpansionStats::default();
    assert_eq!(stats.misses, 0);
    assert_eq!(stats.mapped, 0);
    assert_eq!(stats.new_files, 0);
}

#[test]
fn module_scoped_demand_locates_within_its_package_only() {
    // EXT-1: two packages export the same name. A module-scoped demand must
    // pull only the file inside its own module — never the coincidental
    // same-name symbol in the other package.
    let mut idx = SymbolLocationIndex::new();
    idx.insert("pkg", "find", "/nm/pkg/index.d.ts");
    idx.insert("other", "find", "/nm/other/index.d.ts");

    let miss = ChainMiss {
        current_type: String::new(),
        target_name: "find".to_string(),
        module: Some("pkg".to_string()),
    };
    let hits = locate_via_symbol_index(&idx, &miss);
    assert_eq!(hits, vec![std::path::PathBuf::from("/nm/pkg/index.d.ts")]);
}

#[test]
fn module_scoped_demand_misses_without_cross_package_fallback() {
    // A name absent under its declared module is a genuine gap — no
    // find_by_name fallback that would grep a same-name symbol elsewhere.
    let mut idx = SymbolLocationIndex::new();
    idx.insert("other", "find", "/nm/other/index.d.ts");

    let miss = ChainMiss {
        current_type: String::new(),
        target_name: "find".to_string(),
        module: Some("pkg".to_string()),
    };
    assert!(locate_via_symbol_index(&idx, &miss).is_empty());
}

#[test]
fn module_less_demand_keeps_find_by_name() {
    // A bare/ambient demand (module: None) keeps the existing whole-index
    // find_by_name probe — the EXT-1 change is additive, not a replacement.
    let mut idx = SymbolLocationIndex::new();
    idx.insert("pkg", "find", "/nm/pkg/index.d.ts");
    idx.insert("other", "find", "/nm/other/index.d.ts");

    let miss = ChainMiss {
        current_type: String::new(),
        target_name: "find".to_string(),
        module: None,
    };
    assert_eq!(locate_via_symbol_index(&idx, &miss).len(), 2);
}
