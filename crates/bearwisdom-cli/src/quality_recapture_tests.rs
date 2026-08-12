// Sibling test file for `quality_recapture.rs`. Covers the on-disk states that
// block a capture — the classification that decides whether a project is
// re-indexed, silently preserved, or reported as a failed request.

use super::*;

/// A project root under the OS temp dir, recreated empty for each test.
fn fresh_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create test project root");
    root
}

#[test]
fn absent_path_is_blocked_as_missing() {
    let root = std::env::temp_dir().join("bw-test-recapture-absent");
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(capture_blocker(&root), Some(SkipReason::PathMissing));
}

#[test]
fn root_holding_only_the_index_cache_is_blocked_as_a_ghost() {
    // The state the corpus hit: sources deleted, `.bearwisdom/` left behind.
    // `exists()` is true, so only the non-hidden-entry check catches it.
    let root = fresh_root("bw-test-recapture-ghost");
    std::fs::create_dir_all(root.join(".bearwisdom")).unwrap();
    std::fs::write(root.join(".bearwisdom").join("index.db"), b"stale").unwrap();

    assert!(root.exists(), "ghost roots still exist on disk");
    assert_eq!(capture_blocker(&root), Some(SkipReason::GhostSource));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn root_holding_only_hidden_entries_is_blocked_as_a_ghost() {
    let root = fresh_root("bw-test-recapture-hidden-only");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join(".bearwisdom")).unwrap();

    assert_eq!(capture_blocker(&root), Some(SkipReason::GhostSource));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn root_with_source_is_not_blocked() {
    let root = fresh_root("bw-test-recapture-live");
    std::fs::create_dir_all(root.join(".bearwisdom")).unwrap();
    std::fs::write(root.join("main.rs"), b"fn main() {}").unwrap();

    assert_eq!(capture_blocker(&root), None);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn ghost_root_reported_against_a_scoped_request_becomes_a_failure() {
    // The end-to-end rule: a ghost project named by `--project` must leave a
    // failure behind rather than passing through as a preserved entry.
    let root = fresh_root("bw-test-recapture-scoped-ghost");
    std::fs::create_dir_all(root.join(".bearwisdom")).unwrap();

    let mut requests = RecaptureRequests::new(&["python-posthog".to_string()]);
    assert!(requests.selects("python-posthog"));
    let blocker = capture_blocker(&root).expect("ghost root blocks capture");
    requests.record_skip("python-posthog", &root.to_string_lossy(), blocker);

    let failures = requests.into_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].reason, "ghost_source");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn ghost_root_in_an_unscoped_sweep_leaves_no_failure() {
    let root = fresh_root("bw-test-recapture-unscoped-ghost");
    std::fs::create_dir_all(root.join(".bearwisdom")).unwrap();

    let mut requests = RecaptureRequests::new(&[]);
    assert!(requests.selects("python-posthog"));
    let blocker = capture_blocker(&root).expect("ghost root blocks capture");
    requests.record_skip("python-posthog", &root.to_string_lossy(), blocker);

    assert!(requests.into_failures().is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn civil_from_days_converts_known_epochs() {
    assert_eq!(civil_from_days(0), "1970-01-01");
    // 2000-03-01 — the day after the leap day that trips naive conversions.
    assert_eq!(civil_from_days(11_017), "2000-03-01");
    assert_eq!(civil_from_days(19_723), "2024-01-01");
}

#[test]
fn utc_date_stamp_has_a_fixed_width_shape() {
    let stamp = utc_date_stamp();
    assert_eq!(stamp.len(), 10, "YYYY-MM-DD: {stamp}");
    let parts: Vec<&str> = stamp.split('-').collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0].len(), 4);
    assert_eq!(parts[1].len(), 2);
    assert_eq!(parts[2].len(), 2);
    assert!(stamp.chars().all(|c| c.is_ascii_digit() || c == '-'));
}
