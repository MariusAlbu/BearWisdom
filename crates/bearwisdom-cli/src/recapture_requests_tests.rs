// Sibling test file for `recapture_requests.rs`. Covers the rule that an
// explicitly requested project can never finish a run silently: every name
// that fails to reach fresh metrics leaves a failure behind.

use super::*;

#[test]
fn unscoped_run_selects_everything_and_records_nothing() {
    let mut reqs = RecaptureRequests::new(&[]);
    assert!(!reqs.is_scoped());
    assert!(reqs.selects("go-gitea"));
    assert!(reqs.selects("anything-at-all"));
    // A ghost in a full sweep is tolerated — the entry keeps its old values.
    reqs.record_skip("go-gitea", "F:/x/go-gitea", SkipReason::GhostSource);
    assert!(reqs.into_failures().is_empty());
}

#[test]
fn scoped_run_selects_only_requested_names() {
    let mut reqs = RecaptureRequests::new(&["go-gitea".to_string()]);
    assert!(reqs.is_scoped());
    assert!(reqs.selects("go-gitea"));
    assert!(!reqs.selects("ts-nextjs"));
}

#[test]
fn ghost_skip_of_requested_project_is_a_failure() {
    let mut reqs = RecaptureRequests::new(&["python-posthog".to_string()]);
    assert!(reqs.selects("python-posthog"));
    reqs.record_skip(
        "python-posthog",
        "F:/Work/Projects/TestProjects/python-posthog",
        SkipReason::GhostSource,
    );

    let failures = reqs.into_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].project, "python-posthog");
    assert_eq!(failures[0].reason, "ghost_source");
    assert!(
        failures[0].detail.contains("source tree empty"),
        "detail names the blocking state: {}",
        failures[0].detail
    );
}

#[test]
fn missing_path_skip_of_requested_project_is_a_failure() {
    let mut reqs = RecaptureRequests::new(&["scala-lila".to_string()]);
    assert!(reqs.selects("scala-lila"));
    reqs.record_skip("scala-lila", "F:/gone/scala-lila", SkipReason::PathMissing);

    let failures = reqs.into_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].reason, "path_missing");
    assert!(failures[0].detail.contains("F:/gone/scala-lila"));
}

#[test]
fn requested_name_absent_from_baseline_is_a_failure() {
    // A typo'd or removed project name never reaches the loop body at all;
    // without this the run would exit clean having captured nothing.
    let mut reqs = RecaptureRequests::new(&["ts-nextjs".to_string(), "typo-name".to_string()]);
    assert!(reqs.selects("ts-nextjs"));

    let failures = reqs.into_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].project, "typo-name");
    assert_eq!(failures[0].reason, UNKNOWN_PROJECT);
}

#[test]
fn fully_captured_scoped_run_has_no_failures() {
    let mut reqs = RecaptureRequests::new(&["a".to_string(), "b".to_string()]);
    assert!(reqs.selects("a"));
    assert!(reqs.selects("b"));
    assert!(reqs.into_failures().is_empty());
}

#[test]
fn failures_are_sorted_by_project_name() {
    let mut reqs = RecaptureRequests::new(&[
        "zeta".to_string(),
        "alpha".to_string(),
        "never-listed".to_string(),
    ]);
    assert!(reqs.selects("zeta"));
    assert!(reqs.selects("alpha"));
    reqs.record_skip("zeta", "F:/zeta", SkipReason::GhostSource);
    reqs.record_skip("alpha", "F:/alpha", SkipReason::PathMissing);

    let failures = reqs.into_failures();
    let names: Vec<&str> = failures.iter().map(|f| f.project.as_str()).collect();
    assert_eq!(names, vec!["alpha", "never-listed", "zeta"]);
}

#[test]
fn failures_json_carries_project_reason_and_detail() {
    let mut reqs = RecaptureRequests::new(&["grandnode".to_string()]);
    assert!(reqs.selects("grandnode"));
    reqs.record_skip("grandnode", "F:/grandnode", SkipReason::GhostSource);
    let failures = reqs.into_failures();

    let json = failures_json(&failures);
    let arr = json.as_array().expect("failures serialize as an array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["project"], "grandnode");
    assert_eq!(arr[0]["reason"], "ghost_source");
    assert!(arr[0]["detail"].as_str().unwrap().contains("F:/grandnode"));

    let summary = failure_summary(&failures);
    assert!(summary.contains("grandnode"));
    assert!(summary.contains("ghost_source"));
}
