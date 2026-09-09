use super::*;

#[test]
fn gate_retains_legacy_fields_but_does_not_claim_correctness() {
    let db = Database::open_in_memory().unwrap();
    let report = build(&db).unwrap();
    assert!(report.get("breakdown").is_some());
    assert!(report.get("health").is_some());
    assert!(report["occurrences"]["binding_coverage_percent"].is_null());
    assert!(report["occurrences"]["binding_precision_percent"].is_null());
    assert!(report["occurrences"]["correct_binding_recall_percent"].is_null());
    assert_eq!(
        report["measurement_contract"]["correctness"],
        "requires_independent_ground_truth"
    );
}
