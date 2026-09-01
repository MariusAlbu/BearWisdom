use rustc_hash::FxHashMap;

use super::{apply, compute, MergeGroups};

fn groups_same_file(qname: &str, file: &str, ids: &[i64]) -> MergeGroups {
    let mut g = MergeGroups::default();
    g.by_file
        .insert((qname.to_string(), file.to_string()), ids.to_vec());
    g
}

#[test]
fn min_id_is_canonical_and_singletons_are_ignored() {
    let mut g = groups_same_file("Foo", "a.ts", &[42, 7, 19]);
    g.by_file
        .insert(("Bar".to_string(), "a.ts".to_string()), vec![3]);
    let map = compute(&g);
    assert_eq!(map.get(&42), Some(&7));
    assert_eq!(map.get(&19), Some(&7));
    assert!(!map.contains_key(&7), "canonical maps to itself implicitly");
    assert!(!map.contains_key(&3), "singleton sets never canonicalize");
}

#[test]
fn same_qname_in_different_files_stays_distinct_under_same_file_scope() {
    let mut g = MergeGroups::default();
    g.by_file
        .insert(("Options".to_string(), "a.ts".to_string()), vec![1]);
    g.by_file
        .insert(("Options".to_string(), "b.ts".to_string()), vec![2]);
    assert!(compute(&g).is_empty(), "module scoping keeps them distinct");
}

#[test]
fn apply_unions_member_buckets_and_rewrites_edges() {
    // interface Foo (id 7, members [70]) + namespace Foo (id 42, members [71]);
    // something extends the namespace row (id 42) with args.
    let canonical = compute(&groups_same_file("Foo", "a.ts", &[7, 42]));

    let mut members: FxHashMap<i64, Vec<i64>> = FxHashMap::default();
    members.insert(7, vec![70]);
    members.insert(42, vec![71]);
    let mut inherits: FxHashMap<i64, Vec<i64>> = FxHashMap::default();
    inherits.insert(99, vec![42]); // child 99 extends the namespace row
    let mut args: FxHashMap<(i64, i64), Vec<crate::type_checker::core::types::TypeId>> =
        FxHashMap::default();
    let mut enclosing: FxHashMap<i64, i64> = FxHashMap::default();
    enclosing.insert(71, 42); // the namespace member's enclosing type

    apply(&canonical, &mut members, &mut inherits, &mut args, &mut enclosing);

    let bucket = members.get(&7).expect("canonical bucket");
    assert!(bucket.contains(&70) && bucket.contains(&71), "union: {bucket:?}");
    assert!(!members.contains_key(&42), "non-canonical bucket folded away");
    assert_eq!(inherits.get(&99), Some(&vec![7]), "parent rewritten to canonical");
    assert_eq!(enclosing.get(&71), Some(&7), "enclosing value canonicalized");
}

#[test]
fn apply_is_idempotent() {
    let canonical = compute(&groups_same_file("Foo", "a.ts", &[7, 42]));
    let mut members: FxHashMap<i64, Vec<i64>> = FxHashMap::default();
    members.insert(7, vec![70]);
    members.insert(42, vec![71]);
    let mut inherits: FxHashMap<i64, Vec<i64>> = FxHashMap::default();
    let mut args = FxHashMap::default();
    let mut enclosing: FxHashMap<i64, i64> = FxHashMap::default();

    apply(&canonical, &mut members, &mut inherits, &mut args, &mut enclosing);
    let snapshot = members.clone();
    apply(&canonical, &mut members, &mut inherits, &mut args, &mut enclosing);
    assert_eq!(members, snapshot, "second application changes nothing");
}
