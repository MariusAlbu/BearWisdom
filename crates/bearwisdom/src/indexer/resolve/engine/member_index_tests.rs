use super::*;

#[test]
fn restoring_declared_projection_preserves_appended_source_name_ids() {
    let mut source = MemberIndex::default();
    source.record(1, "declared", 2);
    let canonical = [(1, 1), (2, 2)].into_iter().collect();
    let mut view = source.project(&canonical);
    let appended = view.intern_name("rowless");
    view.project_member(1, appended, vec![3]);
    view.restore_projection(&source, &canonical);
    assert_eq!(view.name("rowless"), Some(appended));
    assert!(!view.declared(1, appended));
    assert_eq!(view.candidates(1, view.name("declared").unwrap()), &[2]);
}

#[test]
fn projected_members_merge_only_attested_owner_ids_and_keep_name_handles() {
    let mut index = MemberIndex::default();
    index.record(1, "first", 11);
    index.record(2, "last", 12);
    index.record(3, "last", 13);
    let name = index.name("last").unwrap();
    let projected = index.project(&[(1, 1), (2, 1), (11, 11), (12, 12)].into_iter().collect());
    assert_eq!(projected.name("last"), Some(name));
    assert_eq!(projected.candidates(1, name), [12]);
    assert!(projected.candidates(2, name).is_empty());
    assert!(projected.candidates(3, name).is_empty());
    assert_eq!(index.candidates(3, name), [13]);
}

#[test]
fn names_are_interned_once_and_candidates_preserve_owner_and_declaration_ids() {
    let mut index = MemberIndex::default();
    index.record(1, "read", 71);
    index.record(1, "read", 72);
    index.record(2, "read", 73);
    index.record(1, "read", 71);
    index.record(1, "write", 74);
    let name = index.name("read").unwrap();
    assert_eq!(index.candidates(1, name), [71, 72]);
    assert_eq!(index.candidates(2, name), [73]);
    assert!(index.candidates(3, name).is_empty());
    assert_ne!(index.name("write"), Some(name));
    assert_eq!(index.name("missing"), None);
}

#[test]
fn rebuilding_members_retains_name_ids_but_removes_stale_rows() {
    let mut index = MemberIndex::default();
    index.record(1, "read", 71);
    let name = index.name("read").unwrap();
    let symbol = super::super::testkit::sym(72, "read", "display.only", "method", "new.rs");
    index.rebuild(
        &[(2, vec![72])].into_iter().collect(),
        &[(72, symbol)].into_iter().collect(),
    );
    assert_eq!(index.name("read"), Some(name));
    assert!(index.candidates(1, name).is_empty());
    assert_eq!(index.candidates(2, name), [72]);
    index.rebuild(&FxHashMap::default(), &FxHashMap::default());
    assert!(index.candidates(2, name).is_empty());
}
