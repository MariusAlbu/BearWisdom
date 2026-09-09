use super::*;
use crate::indexer::resolve::engine::testkit::Lookup;
use crate::type_checker::core::types::{Lifetime, Mutability};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

#[test]
fn projection_is_profile_gated_and_pointer_receivers_never_inherit_pointee_ids() {
    let lookup = Lookup::new();
    let arena = TypeArena::new();
    let target = arena.decl("Same", 41);
    let profile = LanguageProfile {
        reference_member_projection: true,
        ..DEFAULT_PROFILE
    };
    let reference = arena.intern(Type::Indirect {
        kind: Indirection::Reference(Lifetime::Unknown),
        mutability: Mutability::Shared,
        inner: target,
    });
    let projected = project_receiver(
        Receiver::untyped(reference),
        &lookup,
        &arena,
        None,
        &profile,
    );
    assert_eq!((projected.ty, projected.id), (target, Some(41)));
    assert!(matches!(arena.get(reference), Type::Indirect { inner, .. } if inner == target));
    for kind in [
        Indirection::Pointer,
        Indirection::Reference(Lifetime::Static),
    ] {
        let ty = arena.intern(Type::Indirect {
            kind,
            mutability: Mutability::Mutable,
            inner: target,
        });
        let profile = if kind == Indirection::Pointer {
            &profile
        } else {
            &DEFAULT_PROFILE
        };
        let got = project_receiver(Receiver::new(ty, 99), &lookup, &arena, None, profile);
        assert_eq!((got.ty, got.id), (ty, None));
    }
}

#[test]
fn nested_reference_projection_is_bounded() {
    let lookup = Lookup::new();
    let arena = TypeArena::new();
    let mut ty = arena.decl("Same", 41);
    let profile = LanguageProfile {
        reference_member_projection: true,
        ..DEFAULT_PROFILE
    };
    for _ in 0..40 {
        ty = arena.intern(Type::Indirect {
            kind: Indirection::Reference(Lifetime::Static),
            mutability: Mutability::Shared,
            inner: ty,
        });
    }
    let got = project_receiver(Receiver::untyped(ty), &lookup, &arena, None, &profile);
    assert_eq!(arena.get(got.ty), Type::Unknown);
    assert_eq!(got.id, None);
}

#[test]
fn projection_retains_only_the_direct_borrow_and_drops_wrapper_provenance() {
    use crate::type_checker::core::types::{GenericParamData, GenericParamKind};
    let lookup = Lookup::new();
    let arena = TypeArena::new();
    let region = |owner| {
        Lifetime::Parameter(arena.intern_generic(GenericParamData {
            name: "'a".into(),
            kind: GenericParamKind::Lifetime,
            owner_symbol_index: owner,
            bound: None,
        }))
    };
    let outer = region(1);
    let inner = region(2);
    let target = arena.decl("Same", 41);
    let reference = |region, inner| {
        arena.intern(Type::Indirect {
            kind: Indirection::Reference(region),
            mutability: Mutability::Shared,
            inner,
        })
    };
    let direct = reference(inner, target);
    let nested = reference(outer, direct);
    let profile = LanguageProfile {
        reference_member_projection: true,
        single_inner_wrappers: &["Box"],
        ..DEFAULT_PROFILE
    };
    let (recv, borrow) =
        project_with_borrow(Receiver::untyped(nested), &lookup, &arena, None, &profile);
    assert_eq!(recv.ty, target);
    assert_eq!(borrow, Some(direct));
    let wrapped = arena.intern(Type::Apply {
        base: arena.decl("Box", 77),
        args: vec![target],
    });
    let (_, borrow) = project_with_borrow(
        Receiver::untyped(reference(outer, wrapped)),
        &lookup,
        &arena,
        None,
        &profile,
    );
    assert!(
        borrow.is_none(),
        "borrowing a wrapper does not attest to borrowing its pointee"
    );
    let (_, borrow) =
        project_with_borrow(Receiver::untyped(target), &lookup, &arena, None, &profile);
    assert!(
        borrow.is_none(),
        "owned values do not invent an automatic borrow region"
    );
}
