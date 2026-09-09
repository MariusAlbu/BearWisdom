use super::*;
use crate::indexer::{
    resolve::engine::{compilation::Compilation, contract::TypeInfo, testkit::sym},
    symbol_ids::SymbolIds,
};
use crate::type_checker::core::types::{
    GenericParamData, GenericParamKind, Indirection, Lifetime, Mutability,
};
use std::sync::Arc;

#[test]
fn zero_argument_calls_substitute_only_attested_matching_receiver_regions() {
    let arena = Arc::new(TypeArena::new());
    let mut lookup = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    let region = |owner| {
        arena.intern_generic(GenericParamData {
            name: "'a".into(),
            kind: GenericParamKind::Lifetime,
            owner_symbol_index: owner,
            bound: None,
        })
    };
    let method = region(7);
    let caller = region(8);
    let other = region(9);
    let nominal = arena.decl("C", 41);
    let twin = arena.decl("C", 42);
    let reference = |region, mutability, inner| {
        arena.intern(Type::Indirect {
            kind: Indirection::Reference(region),
            mutability,
            inner,
        })
    };
    let yielded = arena.generic_type(method);
    let unknown = arena.intern(Type::Region(Lifetime::Unknown));
    lookup.type_info_by_id.insert(
        7,
        TypeInfo {
            receiver_type_id: Some(reference(
                Lifetime::Parameter(method),
                Mutability::Shared,
                nominal,
            )),
            elided_input_params: vec![(17, 0, method)],
            parameter_type_ids: Some(vec![]),
            return_type_id: Some(yielded),
            ..Default::default()
        },
    );
    let callee = sym(7, "make", "C.make", "method", "lib.rs");
    let correct = reference(Lifetime::Parameter(caller), Mutability::Shared, nominal);
    for (borrowed, expected) in [
        (Some(correct), arena.generic_type(caller)),
        (
            Some(reference(
                Lifetime::Inference {
                    owner: 100,
                    byte: 17,
                },
                Mutability::Shared,
                nominal,
            )),
            arena.intern(Type::Region(Lifetime::Inference {
                owner: 100,
                byte: 17,
            })),
        ),
        (
            Some(reference(Lifetime::Static, Mutability::Shared, nominal)),
            arena.intern(Type::Region(Lifetime::Static)),
        ),
        (None, unknown),
        (
            Some(reference(Lifetime::Unknown, Mutability::Shared, nominal)),
            unknown,
        ),
        (
            Some(reference(
                Lifetime::Parameter(caller),
                Mutability::Shared,
                twin,
            )),
            unknown,
        ),
        (
            Some(reference(
                Lifetime::Parameter(caller),
                Mutability::Mutable,
                nominal,
            )),
            unknown,
        ),
    ] {
        let got = apply_with_receiver(
            &lookup,
            &arena,
            &callee,
            0,
            &[],
            &[],
            nominal,
            Some(41),
            Some(yielded),
            &[],
            borrowed,
        );
        assert_eq!(got, Some(expected));
    }
    // An ordinary argument with the same display name owns a DIFFERENT ID.
    lookup
        .type_info_by_id
        .get_mut(&7)
        .unwrap()
        .parameter_type_ids = Some(vec![reference(
        Lifetime::Parameter(other),
        Mutability::Shared,
        nominal,
    )]);
    lookup
        .type_info_by_id
        .get_mut(&7)
        .unwrap()
        .elided_input_params
        .push((27, 0, other));
    let actual = reference(Lifetime::Static, Mutability::Shared, nominal);
    let env = bound_call::environment_with_receiver(
        &lookup,
        &arena,
        &callee,
        nominal,
        Some(41),
        &[],
        &[actual],
        Some(correct),
    )
    .unwrap();
    assert_eq!(env[&method], arena.generic_type(caller));
    assert_eq!(env[&other], arena.intern(Type::Region(Lifetime::Static)));
    let env = bound_call::environment_with_receiver(
        &lookup,
        &arena,
        &callee,
        nominal,
        Some(41),
        &[],
        &[actual],
        None,
    )
    .unwrap();
    assert_eq!(
        env[&method], unknown,
        "ordinary input must not impersonate receiver evidence"
    );
}
