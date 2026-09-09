use super::*;

#[test]
fn source_projection_revokes_display_aliases_and_retains_all_candidates() {
    let mut members = MemberIndex::default();
    members.record(1, "display", 10);
    members.record(1, "display", 11);
    members.record(2, "display", 20);
    let display = members.name("display").unwrap();
    let actual = members.intern_name("actual");
    let rowless = members.intern_name("rowless");
    let unrelated = members.intern_name("unrelated");
    members.project_member(1, unrelated, vec![]);
    let index = Index(FxHashMap::from_iter([(
        1,
        FxHashMap::from_iter([(actual, vec![10, 11]), (rowless, vec![])]),
    )]));
    index.project(&mut members);
    assert!(members.candidates(1, display).is_empty());
    assert_eq!(members.candidates(1, actual), [10, 11]);
    assert!(members.declared(1, rowless));
    assert!(
        members.declared(1, unrelated),
        "uncovered rowless evidence must survive projection"
    );
    assert_eq!(members.candidates(2, display), [20]);
    index.project(&mut members);
    assert_eq!(members.candidates(1, actual), [10, 11]);
}
