use super::*;

#[test]
fn oracle_contract_round_trips_without_names_or_database_ids() {
    let report = evaluate(CorpusRevision([7; 32]), &[], &[]).unwrap();
    let encoded = serde_json::to_string(&report).unwrap();
    assert_eq!(
        serde_json::from_str::<OracleReport>(&encoded).unwrap(),
        report
    );
    assert_eq!(report.counts.binding_precision_percent, None);
    assert_eq!(report.counts.correct_binding_recall_percent, None);
    assert_eq!(report.counts.extraction_coverage_percent, None);
}
