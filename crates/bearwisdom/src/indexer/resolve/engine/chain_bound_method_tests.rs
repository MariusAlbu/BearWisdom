// Source-target and return-ID integration gates are kept in the compiler oracle.
use super::*;
use crate::indexer::resolve::engine::testkit::{sym, Lookup};
use crate::type_checker::core::types::{Indirection, Lifetime, Mutability};

#[test]
fn optional_call_projection_preserves_non_null_identity_and_unknown_barriers() {
    use crate::type_checker::core::types::Intrinsic;
    let arena = TypeArena::new();
    let lookup = Lookup::new();
    let number = arena.intern(Type::Intrinsic(Intrinsic::Number));
    let text = arena.intern(Type::Intrinsic(Intrinsic::String));
    let missing = arena.intern(Type::Intrinsic(Intrinsic::Undefined));
    assert_eq!(
        optional_receiver(
            &lookup,
            &arena,
            arena.intern(Type::Union(vec![number, missing]))
        ),
        Some((number, true))
    );
    assert_eq!(
        optional_receiver(&lookup, &arena, arena.intern(Type::Optional(number))),
        Some((number, true))
    );
    assert_eq!(
        optional_receiver(&lookup, &arena, number),
        Some((number, false))
    );
    assert_eq!(
        optional_receiver(
            &lookup,
            &arena,
            arena.intern(Type::Union(vec![number, text, missing]))
        ),
        None
    );
    assert_eq!(
        optional_receiver(
            &lookup,
            &arena,
            arena.intern(Type::Union(vec![number, arena.intern(Type::Unknown)]))
        ),
        None
    );
}

#[test]
fn head_attestation_preserves_arguments_and_all_original_reference_layers() {
    let arena = TypeArena::new();
    let lookup = Lookup::new().with(sym(3, "Container", "Container", "struct", "lib.rs"));
    let arg = arena.decl("Doc", 4);
    let value = arena.intern(Type::Apply {
        base: arena.class("Container"),
        args: vec![arg],
    });
    let reference = |inner, mutability| {
        arena.intern(Type::Indirect {
            kind: Indirection::Reference(Lifetime::Static),
            mutability,
            inner,
        })
    };
    let original = reference(reference(value, Mutability::Mutable), Mutability::Shared);
    let expected = arena.intern(Type::Apply {
        base: arena.decl("Container", 3),
        args: vec![arg],
    });
    assert_eq!(
        receiver(
            &lookup,
            &arena,
            original,
            Receiver {
                ty: value,
                id: Some(3)
            }
        ),
        reference(reference(expected, Mutability::Mutable), Mutability::Shared)
    );
}

#[test]
fn wrapper_projection_cannot_stamp_its_pointee_id_on_the_original_application() {
    let arena = TypeArena::new();
    let lookup = Lookup::new().with(sym(3, "Doc", "Doc", "struct", "lib.rs"));
    let arg = arena.decl("Doc", 3);
    let wrapper = arena.intern(Type::Apply {
        base: arena.class("Wrapper"),
        args: vec![arg],
    });
    assert_eq!(
        receiver(
            &lookup,
            &arena,
            wrapper,
            Receiver {
                ty: arg,
                id: Some(3)
            }
        ),
        wrapper
    );
    let other = arena.decl("Doc", 4);
    assert_eq!(
        receiver(
            &lookup,
            &arena,
            other,
            Receiver {
                ty: other,
                id: Some(3)
            }
        ),
        other,
        "bound identity cannot be replaced by a namesake"
    );
}

#[test]
fn selected_method_environment_preserves_distinct_generic_ids() {
    use crate::type_checker::core::types::*;
    let arena = TypeArena::new();
    let a = arena.intern_generic(GenericParamData {
        name: "T".into(),
        owner_symbol_index: 0,
        bound: None,
        kind: GenericParamKind::Type,
    });
    let b = arena.intern_generic(GenericParamData {
        name: "T".into(),
        owner_symbol_index: 1,
        bound: None,
        kind: GenericParamKind::Type,
    });
    let concrete = arena.decl("Doc", 3);
    let env = [(a, concrete)].into_iter().collect();
    assert_eq!(
        crate::indexer::resolve::engine::contract::generic_return::substitute(
            &arena,
            arena.generic_type(a),
            &env
        ),
        concrete
    );
    assert_eq!(
        crate::indexer::resolve::engine::contract::generic_return::substitute(
            &arena,
            arena.generic_type(b),
            &env
        ),
        arena.generic_type(b)
    );
}
