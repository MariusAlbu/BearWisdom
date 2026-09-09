use super::*;
use crate::indexer::resolve::engine::testkit::Lookup;
use crate::type_checker::core::types::Mutability;

#[test]
fn open_regions_bind_but_different_rigid_ids_and_unknown_regions_are_not_equality_evidence() {
    use crate::type_checker::core::types::{GenericParamData, GenericParamKind};
    let lookup = Lookup::new();
    let arena = TypeArena::new();
    let parameter = |owner| {
        arena.intern_generic(GenericParamData {
            name: "'a".into(),
            kind: GenericParamKind::Lifetime,
            owner_symbol_index: owner,
            bound: None,
        })
    };
    let p = parameter(1);
    let a = parameter(2);
    let b = parameter(3);
    let pair = |a, b| {
        arena.intern(Type::Tuple(vec![
            arena.intern(Type::Region(a)),
            arena.intern(Type::Region(b)),
        ]))
    };
    let pattern = ReceiverPattern {
        ty: pair(Lifetime::Parameter(p), Lifetime::Parameter(p)),
        parameters: vec![p],
    };
    assert_eq!(
        pattern
            .bindings(
                &lookup,
                &arena,
                pair(Lifetime::Parameter(a), Lifetime::Parameter(a))
            )
            .unwrap()
            .unwrap()[&p],
        arena.generic_type(a)
    );
    assert!(matches!(
        pattern.bindings(&lookup, &arena, pair(Lifetime::Static, Lifetime::Static)),
        Ok(Some(_))
    ));
    for (a, b) in [
        (Lifetime::Parameter(a), Lifetime::Parameter(b)),
        (Lifetime::Parameter(a), Lifetime::Static),
        (Lifetime::Unknown, Lifetime::Unknown),
    ] {
        assert!(pattern.bindings(&lookup, &arena, pair(a, b)).is_err());
    }
    let rigid = ReceiverPattern {
        ty: arena.generic_type(a),
        parameters: vec![],
    };
    assert!(rigid
        .bindings(&lookup, &arena, arena.generic_type(b))
        .is_err());
    assert_eq!(
        rigid.bindings(&lookup, &arena, arena.decl("'a", 88)),
        Ok(None)
    );
}

#[test]
fn indirection_keeps_kind_mutability_and_referent_ids_and_unknown_regions_are_not_proof() {
    let lookup = Lookup::new();
    let arena = TypeArena::new();
    let a = arena.decl("Same", 41);
    let b = arena.decl("Same", 42);
    let indirect = |kind, mutability, inner| {
        arena.intern(Type::Indirect {
            kind,
            mutability,
            inner,
        })
    };
    let shared = Mutability::Shared;
    let mutable = Mutability::Mutable;
    let static_ref = Indirection::Reference(Lifetime::Static);
    let pattern = ReceiverPattern {
        ty: indirect(static_ref, shared, a),
        parameters: vec![],
    };
    assert!(matches!(
        pattern.bindings(&lookup, &arena, pattern.ty),
        Ok(Some(_))
    ));
    for ty in [
        a,
        indirect(static_ref, mutable, a),
        indirect(Indirection::Pointer, shared, a),
        indirect(static_ref, shared, b),
    ] {
        assert_eq!(pattern.bindings(&lookup, &arena, ty), Ok(None));
    }
    let unresolved = indirect(Indirection::Reference(Lifetime::Unknown), shared, a);
    assert!(pattern.bindings(&lookup, &arena, unresolved).is_err());
    let unknown = ReceiverPattern {
        ty: unresolved,
        parameters: vec![],
    };
    assert!(unknown.bindings(&lookup, &arena, unresolved).is_err());
    let nested = indirect(static_ref, shared, arena.intern(Type::Unknown));
    assert!(pattern.bindings(&lookup, &arena, nested).is_err());
}

#[test]
fn exact_ids_filter_fixed_arguments_and_reject_unknown_identity() {
    let lookup = Lookup::new();
    let arena = TypeArena::new();
    let owner = arena.decl("Owner", 10);
    let application = |arg| {
        arena.intern(Type::Apply {
            base: owner,
            args: vec![arg],
        })
    };
    let pattern = ReceiverPattern {
        ty: application(arena.primitive(PrimKind::Signed(32))),
        parameters: vec![],
    };
    assert!(matches!(
        pattern.bindings(&lookup, &arena, pattern.ty),
        Ok(Some(_))
    ));
    assert_eq!(
        pattern.bindings(
            &lookup,
            &arena,
            application(arena.primitive(PrimKind::Unsigned(32)))
        ),
        Ok(None)
    );
    assert!(pattern
        .bindings(&lookup, &arena, application(arena.primitive(PrimKind::Int)))
        .is_err());
    assert!(pattern
        .bindings(&lookup, &arena, application(arena.class("i32")))
        .is_err());
    assert!(pattern.bindings(&lookup, &arena, owner).is_err());
}

#[test]
fn parameters_bind_by_id_and_repeated_patterns_require_agreement() {
    let lookup = Lookup::new();
    let arena = TypeArena::new();
    let param = arena.intern_generic(crate::type_checker::core::types::GenericParamData {
        kind: Default::default(),
        name: "T".into(),
        owner_symbol_index: 0,
        bound: None,
    });
    let generic = arena.intern(Type::Generic { param });
    let a = arena.decl("same.display", 11);
    let b = arena.decl("same.display", 12);
    let pair = |a, b| arena.intern(Type::Tuple(vec![a, b]));
    let pattern = ReceiverPattern {
        ty: pair(generic, generic),
        parameters: vec![param],
    };
    assert_eq!(
        pattern
            .bindings(&lookup, &arena, pair(a, a))
            .unwrap()
            .unwrap()[&param],
        a
    );
    assert_eq!(pattern.bindings(&lookup, &arena, pair(a, b)), Ok(None));
    let unknown = arena.intern(Type::Unknown);
    assert!(pattern
        .bindings(&lookup, &arena, pair(unknown, unknown))
        .is_err());
    let unbound = arena.class("same.display");
    assert!(pattern
        .bindings(&lookup, &arena, pair(unbound, unbound))
        .is_err());
    assert!(matches!(
        pattern.bindings(&lookup, &arena, pair(generic, generic)),
        Ok(Some(_))
    ));
}

#[test]
fn nested_application_work_is_bounded() {
    let lookup = Lookup::new();
    let arena = TypeArena::new();
    let owner = arena.decl("Owner", 10);
    let mut ty = owner;
    for _ in 0..40 {
        ty = arena.intern(Type::Apply {
            base: owner,
            args: vec![ty],
        });
    }
    assert!(expand(&lookup, &arena, ty).is_none());
    let wide = arena.intern(Type::Tuple(vec![owner; 4097]));
    assert!(expand(&lookup, &arena, wide).is_none());
}
