use super::*;

fn edge(source: &str, target: &str, kind: &str) -> EdgeKey {
    EdgeKey {
        source: source.to_string(),
        target: target.to_string(),
        kind: kind.to_string(),
    }
}

fn set(edges: &[EdgeKey]) -> HashSet<EdgeKey> {
    edges.iter().cloned().collect()
}

#[test]
fn identical_sets_have_no_regressions_or_gains() {
    let a = set(&[edge("A.f", "B.g", "calls"), edge("A.f", "C", "type_ref")]);
    let (regressions, gains) = diff_sets(&a, &a.clone());
    assert!(regressions.is_empty());
    assert!(gains.is_empty());
}

#[test]
fn regression_is_edge_in_legacy_not_in_engine() {
    let legacy = set(&[edge("A.f", "B.g", "calls"), edge("A.f", "C", "type_ref")]);
    let engine = set(&[edge("A.f", "B.g", "calls")]);
    let (regressions, gains) = diff_sets(&legacy, &engine);
    assert_eq!(regressions, vec![edge("A.f", "C", "type_ref")]);
    assert!(gains.is_empty());
}

#[test]
fn gain_is_edge_in_engine_not_in_legacy() {
    let legacy = set(&[edge("A.f", "B.g", "calls")]);
    let engine = set(&[edge("A.f", "B.g", "calls"), edge("A.f", "D.h", "calls")]);
    let (regressions, gains) = diff_sets(&legacy, &engine);
    assert!(regressions.is_empty());
    assert_eq!(gains, vec![edge("A.f", "D.h", "calls")]);
}

#[test]
fn regressions_are_sorted_by_source_then_target_then_kind() {
    let legacy = set(&[
        edge("Z.f", "B.g", "calls"),
        edge("A.f", "Y.g", "calls"),
        edge("A.f", "X.g", "type_ref"),
    ]);
    let engine = HashSet::new();
    let (regressions, _gains) = diff_sets(&legacy, &engine);
    assert_eq!(
        regressions,
        vec![
            edge("A.f", "X.g", "type_ref"),
            edge("A.f", "Y.g", "calls"),
            edge("Z.f", "B.g", "calls"),
        ]
    );
}

#[test]
fn at_parity_when_engine_covers_every_legacy_edge() {
    let covered = ResolveDiff {
        legacy_edges: 2,
        engine_edges: 3,
        regressions: vec![],
        gains: vec![edge("A.f", "D.h", "calls")],
    };
    assert!(covered.at_parity());

    let gap = ResolveDiff {
        legacy_edges: 2,
        engine_edges: 1,
        regressions: vec![edge("A.f", "C", "type_ref")],
        gains: vec![],
    };
    assert!(!gap.at_parity());
}
