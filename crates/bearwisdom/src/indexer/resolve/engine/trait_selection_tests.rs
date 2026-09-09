use super::*;

#[test]
fn candidate_reference_retains_mutability_and_call_identity() {
    let arena = TypeArena::new();
    let inner = arena.decl("Doc", 7);
    let region = Lifetime::Inference { owner: 9, byte: 42 };
    let shared = reference(&arena, inner, region, Mutability::Shared);
    let mutable = reference(&arena, inner, region, Mutability::Mutable);
    assert_ne!(shared, mutable);
    assert!(
        matches!(arena.get(shared), Type::Indirect { kind: Indirection::Reference(r), inner: i, .. } if r == region && i == inner)
    );
}
