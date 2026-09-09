use super::*;

#[test]
fn annotation_regions_cannot_be_refined_through_a_foreign_program_nominal() {
    use crate::type_checker::core::types::{Mutability, NominalContextId};
    let arena = TypeArena::new();
    let a = NominalContextId::fresh();
    let b = NominalContextId::fresh();
    let own = arena.decl_in(a, "Doc", 71);
    let foreign = arena.decl_in(b, "Doc", 71);
    let reference = |region, inner| {
        arena.intern(Type::Indirect {
            kind: Indirection::Reference(region),
            mutability: Mutability::Shared,
            inner,
        })
    };
    assert!(refine_regions(
        &arena,
        reference(Lifetime::Unknown, own),
        reference(Lifetime::Static, foreign),
        0
    )
    .is_none());
    assert!(refine_regions(
        &arena,
        reference(Lifetime::Unknown, own),
        reference(Lifetime::Static, own),
        0
    )
    .is_some());
}
use crate::type_checker::core::types::Mutability;
use crate::types::SourceSpan;

#[test]
fn annotation_refinement_preserves_nominal_mutability_and_known_region_constraints() {
    let arena = TypeArena::new();
    let one = arena.decl("same", 71);
    let two = arena.decl("same", 72);
    let same_id = arena.decl("different_display", 71);
    let region = Lifetime::Inference {
        owner: 99,
        byte: 31,
    };
    let reference = |region, mutability, inner| {
        arena.intern(Type::Indirect {
            kind: Indirection::Reference(region),
            mutability,
            inner,
        })
    };
    let declared = reference(Lifetime::Unknown, Mutability::Shared, one);
    let actual = reference(region, Mutability::Shared, same_id);
    let refined = refine_regions(&arena, declared, actual, 0).unwrap();
    assert_eq!(refined, reference(region, Mutability::Shared, one));
    assert_eq!(
        refine_regions(
            &arena,
            declared,
            reference(region, Mutability::Shared, two),
            0
        ),
        None
    );
    assert_eq!(
        refine_regions(
            &arena,
            declared,
            reference(region, Mutability::Mutable, one),
            0
        ),
        None
    );
    assert_eq!(
        refine_regions(
            &arena,
            reference(Lifetime::Static, Mutability::Shared, one),
            actual,
            0
        ),
        None
    );
    let unknown = arena.intern(Type::Unknown);
    assert_eq!(
        refine_regions(
            &arena,
            reference(Lifetime::Unknown, Mutability::Shared, unknown),
            reference(region, Mutability::Shared, unknown),
            0
        ),
        None
    );
    assert_eq!(
        refine_regions(
            &arena,
            reference(Lifetime::Unknown, Mutability::Shared, declared),
            reference(region, Mutability::Shared, actual),
            0
        ),
        Some(reference(region, Mutability::Shared, refined))
    );
}

#[test]
fn owner_lowering_and_nested_borrow_evaluation_preserve_exact_ids_and_read_positions() {
    let arena = TypeArena::new();
    let input = arena.decl("same", 71);
    let read = ValueExpr::Read {
        binding: BindingId(4),
        byte: 29,
    };
    let shared = ValueExpr::Borrow {
        owner: 8,
        span: SourceSpan { start: 28, end: 30 },
        mutability: Mutability::Shared,
        operand: Box::new(read),
    };
    let nested = ValueExpr::Borrow {
        owner: 8,
        span: SourceSpan { start: 27, end: 30 },
        mutability: Mutability::Mutable,
        operand: Box::new(shared),
    };
    let lowered = lower(&nested, &|slot| (slot == 8).then_some(88), &|_| None, 0);
    let inner = arena.intern(Type::Indirect {
        kind: Indirection::Reference(Lifetime::Inference {
            owner: 88,
            byte: 28,
        }),
        mutability: Mutability::Shared,
        inner: input,
    });
    let expected = arena.intern(Type::Indirect {
        kind: Indirection::Reference(Lifetime::Inference {
            owner: 88,
            byte: 27,
        }),
        mutability: Mutability::Mutable,
        inner,
    });
    assert_eq!(
        evaluate(&lowered, &arena, 0, None, &|binding, byte, _| {
            assert_eq!((binding, byte), (BindingId(4), 29));
            Some(input)
        }),
        Some(expected)
    );
    assert_eq!(lower(&nested, &|_| None, &|_| None, 0), ValueExpr::Unknown);
    assert_eq!(
        evaluate(&lowered, &arena, 0, None, &|_, _, _| None),
        Some(arena.intern(Type::Unknown))
    );
    assert_eq!(
        evaluate(&lowered, &arena, 0, None, &|_, _, _| Some(
            arena.class("same")
        )),
        Some(arena.intern(Type::Unknown))
    );
    assert_eq!(
        lower(&nested, &|_| Some(88), &|_| None, 32),
        ValueExpr::Unknown
    );
    assert_eq!(
        evaluate(&lowered, &arena, 128, None, &|_, _, _| panic!(
            "depth limit must not read"
        )),
        Some(arena.intern(Type::Unknown))
    );
}
