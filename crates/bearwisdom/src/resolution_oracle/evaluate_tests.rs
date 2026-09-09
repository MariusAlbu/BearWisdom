use super::*;

fn site(byte_offset: u32) -> ReferenceSite {
    ReferenceSite {
        file: FixtureFileId(1),
        byte_offset,
        kind: EdgeKind::Calls,
    }
}
fn target(line: u32) -> DeclarationSite {
    DeclarationSite {
        file: FixtureFileId(1),
        line,
        col: 0,
        kind: SymbolKind::Method,
    }
}
fn label(offset: u32, declaration: Option<u32>) -> ExpectedReference {
    ExpectedReference {
        site: site(offset),
        target: declaration.map(target),
    }
}
fn observation(offset: u32, binding: Option<ObservedBinding>) -> Observation {
    Observation {
        site: site(offset),
        binding,
    }
}

#[test]
fn wrong_bindings_and_missing_extraction_lower_recall_independently() {
    let expected: Vec<_> = (0..6).map(|i| label(i, Some(1))).collect();
    let report = evaluate(
        CorpusRevision([1; 32]),
        &expected,
        &[
            observation(0, Some(ObservedBinding::Resolved(target(1)))),
            observation(1, Some(ObservedBinding::Resolved(target(2)))),
            observation(2, Some(ObservedBinding::Unresolved)),
            observation(3, Some(ObservedBinding::Drained)),
            observation(4, None), // 5 was never extracted.
            observation(90, Some(ObservedBinding::Resolved(target(1)))), // No label.
        ],
    )
    .unwrap();
    assert_eq!(report.counts.labelled, 6);
    assert_eq!(report.counts.correct, 1);
    assert_eq!(report.counts.incorrect, 1);
    assert_eq!(report.counts.unresolved, 2);
    assert_eq!(report.counts.missing_resolution, 1);
    assert_eq!(report.counts.not_extracted, 1);
    assert_eq!(report.counts.unlabelled_observations, 1);
    assert_eq!(report.counts.binding_precision_percent, Some(50.0));
    assert_eq!(
        report.counts.correct_binding_recall_percent,
        Some(100.0 / 6.0)
    );
}

#[test]
fn negatives_do_not_inflate_positive_recall_and_false_positives_hurt_precision() {
    let report = evaluate(
        CorpusRevision([1; 32]),
        &[label(0, Some(1)), label(1, None), label(2, None)],
        &[
            observation(0, Some(ObservedBinding::Resolved(target(1)))),
            observation(1, Some(ObservedBinding::Unresolved)),
            observation(2, Some(ObservedBinding::Resolved(target(1)))),
        ],
    )
    .unwrap();
    assert_eq!(report.counts.correct_unbound, 1);
    assert_eq!(report.counts.binding_precision_percent, Some(50.0));
    assert_eq!(report.counts.correct_binding_recall_percent, Some(100.0));
}

#[test]
fn same_spelling_in_another_file_is_a_different_declaration() {
    let mut foreign = target(1);
    foreign.file = FixtureFileId(2);
    let report = evaluate(
        CorpusRevision([1; 32]),
        &[label(0, Some(1))],
        &[observation(0, Some(ObservedBinding::Resolved(foreign)))],
    )
    .unwrap();
    assert_eq!(report.counts.incorrect, 1);
}

#[test]
fn dangling_target_is_not_an_honest_unresolved_result() {
    let report = evaluate(
        CorpusRevision([1; 32]),
        &[label(0, Some(1))],
        &[observation(0, Some(ObservedBinding::DanglingTarget))],
    )
    .unwrap();
    assert_eq!(report.counts.incorrect, 1);
    assert_eq!(report.counts.binding_precision_percent, Some(0.0));
}

#[test]
fn duplicate_labels_or_observations_are_rejected() {
    assert!(evaluate(
        CorpusRevision([1; 32]),
        &[label(0, Some(1)), label(0, Some(2))],
        &[]
    )
    .is_err());
    assert!(evaluate(
        CorpusRevision([1; 32]),
        &[],
        &[observation(0, None), observation(0, None)]
    )
    .is_err());
}

#[test]
fn equal_totals_cannot_hide_retargeting_or_compensating_regressions() {
    let labels = [label(0, Some(1)), label(1, Some(1))];
    let before = evaluate(
        CorpusRevision([1; 32]),
        &labels,
        &[
            observation(0, Some(ObservedBinding::Resolved(target(1)))),
            observation(1, Some(ObservedBinding::Resolved(target(2)))),
        ],
    )
    .unwrap();
    let after = evaluate(
        CorpusRevision([1; 32]),
        &labels,
        &[
            observation(0, Some(ObservedBinding::Resolved(target(2)))),
            observation(1, Some(ObservedBinding::Resolved(target(1)))),
        ],
    )
    .unwrap();
    assert_eq!(before.counts, after.counts);
    let changes = compare(&before, &after).unwrap();
    assert_eq!(changes.len(), 2);
    assert!(changes[0].regressed && changes[0].retargeted);
    assert!(!changes[1].regressed && changes[1].retargeted);
}

#[test]
fn changing_ground_truth_or_revision_requires_an_explicit_rebaseline() {
    let before = evaluate(CorpusRevision([1; 32]), &[label(0, Some(1))], &[]).unwrap();
    let changed_label = evaluate(CorpusRevision([1; 32]), &[label(0, Some(2))], &[]).unwrap();
    let changed_revision = evaluate(CorpusRevision([2; 32]), &[label(0, Some(1))], &[]).unwrap();
    assert!(compare(&before, &changed_label).is_err());
    assert!(compare(&before, &changed_revision).is_err());
}

#[test]
fn equivalent_unbound_outcomes_are_changes_but_not_regressions() {
    let labels = [label(0, None)];
    let before = evaluate(
        CorpusRevision([1; 32]),
        &labels,
        &[observation(0, Some(ObservedBinding::Unresolved))],
    )
    .unwrap();
    let after = evaluate(
        CorpusRevision([1; 32]),
        &labels,
        &[observation(0, Some(ObservedBinding::Drained))],
    )
    .unwrap();
    let changes = compare(&before, &after).unwrap();
    assert_eq!(changes.len(), 1);
    assert!(!changes[0].regressed);
    assert!(!changes[0].retargeted);
}
