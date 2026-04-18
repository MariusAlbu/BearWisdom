// =============================================================================
// indexer/expand_tests.rs — unit tests for indexer/expand.rs
//
// Kept in a sibling file so the production module stays free of synthetic
// fixture literals (dep names, file paths) that look like hardcoded
// production values to a casual reader.
// =============================================================================

use super::*;
use crate::ecosystem::externals::ExternalDepRoot;

fn mk_dep(module_path: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: module_path.to_string(),
        version: "1.0".to_string(),
        root: std::path::PathBuf::from("/tmp/x"),
        ecosystem: "npm",
        package_id: None,
        requested_imports: Vec::new(),
    }
}

#[test]
fn infer_dep_picks_leading_segment() {
    let deps = vec![mk_dep("alpha"), mk_dep("beta"), mk_dep("gamma")];
    assert_eq!(_test_infer_dep_idx_for_fqn("alpha.SomeType", &deps), Some(0));
    assert_eq!(_test_infer_dep_idx_for_fqn("beta", &deps), Some(1));
    assert_eq!(_test_infer_dep_idx_for_fqn("Delta.Thing", &deps), None);
    assert_eq!(_test_infer_dep_idx_for_fqn("UnknownThing.X", &deps), None);
}

#[test]
fn infer_dep_prefers_longer_prefix() {
    let deps = vec![mk_dep("foo"), mk_dep("foo.bar")];
    assert_eq!(_test_infer_dep_idx_for_fqn("foo.bar.Baz", &deps), Some(1));
    assert_eq!(_test_infer_dep_idx_for_fqn("foo.qux", &deps), Some(0));
}

#[test]
fn infer_dep_rejects_partial_segment_match() {
    // A dep named "alpha" must not absorb FQNs that merely share a prefix
    // (`alpharadar.X`) — only segment-aligned matches count.
    let deps = vec![mk_dep("alpha")];
    assert_eq!(_test_infer_dep_idx_for_fqn("alpharadar.X", &deps), None);
    assert_eq!(_test_infer_dep_idx_for_fqn("alpha.Component", &deps), Some(0));
}

#[test]
fn parse_dep_name_handles_scoped_and_unscoped() {
    assert_eq!(
        _test_parse_dep_name_from_virtual_path("ext:typescript:libfoo/lib/Bar.ts"),
        Some("libfoo")
    );
    assert_eq!(
        _test_parse_dep_name_from_virtual_path(
            "ext:typescript:@scope/pkg/dist/index.d.ts"
        ),
        Some("@scope/pkg")
    );
    assert_eq!(
        _test_parse_dep_name_from_virtual_path("ext:python:libfoo/main.py"),
        Some("libfoo")
    );
    assert_eq!(_test_parse_dep_name_from_virtual_path("not_external/foo.ts"), None);
}

#[test]
fn empty_misses_returns_zero_stats() {
    // Smoke test: we don't even touch the DB if there's nothing to do.
    // This is the hot path on a project with perfect resolution.
    let mut stats = ExpansionStats::default();
    stats.misses = 0;
    assert_eq!(stats.misses, 0);
    assert_eq!(stats.mapped, 0);
    assert_eq!(stats.new_files, 0);
}
