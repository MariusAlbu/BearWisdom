use super::*;
#[test]
fn exact_scalars_are_distinct_and_serializable() {
    let arena = crate::type_checker::core::types::TypeArena::new();
    let kinds = [
        PrimKind::Int,
        PrimKind::Signed(32),
        PrimKind::Unsigned(32),
        PrimKind::Signed(64),
        PrimKind::Isize,
        PrimKind::Usize,
        PrimKind::Float,
        PrimKind::FloatWidth(32),
        PrimKind::FloatWidth(64),
    ];
    let ids: std::collections::HashSet<_> = kinds.iter().map(|&p| arena.primitive(p)).collect();
    assert_eq!(ids.len(), kinds.len());
    for p in kinds {
        assert_eq!(
            serde_json::from_str::<PrimKind>(&serde_json::to_string(&p).unwrap()).unwrap(),
            p
        );
    }
}
