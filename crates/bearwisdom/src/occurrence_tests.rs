use super::*;

#[test]
fn partition_counts_skips_but_never_conflates_drains_with_resolutions() {
    let mut counts = OccurrenceCounts::default();
    for disposition in [
        Disposition::Resolved,
        Disposition::Unresolved,
        Disposition::Drained,
        Disposition::Primitive,
        Disposition::Duplicate,
        Disposition::MissingSourceSymbol,
        Disposition::MissingSourceId,
        Disposition::UnsupportedLanguage,
    ] {
        counts.add(disposition, 1);
    }
    assert_eq!(counts.total(), 8);
    assert_eq!(counts.skipped(), 3);
    assert_eq!(counts.binding_coverage_percent(), Some(20.0));
}

#[test]
fn empty_or_entirely_excluded_input_has_no_coverage_percentage() {
    let mut counts = OccurrenceCounts::default();
    assert_eq!(counts.binding_coverage_percent(), None);
    counts.add(Disposition::Drained, 20);
    counts.add(Disposition::Primitive, 30);
    counts.add(Disposition::Duplicate, 2);
    assert_eq!(counts.binding_coverage_percent(), None);
}
