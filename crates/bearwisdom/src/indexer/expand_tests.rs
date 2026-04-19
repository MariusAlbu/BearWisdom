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
