use super::*;

#[test]
fn unknown_bound_and_cycles_are_not_success() {
    let arena = TypeArena::new();
    let lookup = super::super::super::testkit::Lookup::new();
    let graph = Graph::default();
    let bound = Obligation {
        subject: arena.intern(Type::Unknown),
        trait_type: arena.intern(Type::Unknown),
    };
    assert_eq!(
        prove(
            &graph,
            &lookup,
            &arena,
            bound,
            &[],
            &mut FxHashSet::default(),
            &mut 32
        ),
        Err(())
    );
    assert_eq!(
        prove(
            &graph,
            &lookup,
            &arena,
            bound,
            &[bound],
            &mut FxHashSet::default(),
            &mut 32
        ),
        Err(()),
        "unknown is not a reflexive proof"
    );
}

#[test]
fn a_deferred_type_operator_is_not_an_applicability_proof() {
    use crate::type_checker::core::types::{Intrinsic, TypeOperator};
    let arena = TypeArena::new();
    for inner in [
        arena.intern(Type::Unknown),
        arena.intern(Type::Intrinsic(Intrinsic::String)),
    ] {
        let op = arena.intern(Type::Operator(TypeOperator::KeyOf(inner)));
        assert!(!known(&arena, op));
        assert!(!known(&arena, arena.intern(Type::Tuple(vec![op]))));
    }
}
