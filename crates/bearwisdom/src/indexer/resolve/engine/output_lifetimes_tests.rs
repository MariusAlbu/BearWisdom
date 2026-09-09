use super::super::testkit::{sym, Lookup};
use super::*;
use crate::type_checker::core::types::{GenericParamData, GenericParamKind, Mutability};

#[test]
fn unique_source_position_carries_its_id_and_repeated_or_unknown_positions_are_not_evidence() {
    let arena = TypeArena::new();
    let lookup = Lookup::new().with(sym(42, "Doc", "Doc", "struct", "lib.rs"));
    let region = || {
        arena.intern_generic(GenericParamData {
            name: "'a".into(),
            kind: GenericParamKind::Lifetime,
            owner_symbol_index: 0,
            bound: None,
        })
    };
    let a = Lifetime::Parameter(region());
    let b = Lifetime::Parameter(region());
    let reference = |r| {
        arena.intern(Type::Indirect {
            kind: Indirection::Reference(r),
            mutability: Mutability::Shared,
            inner: arena.decl("Doc", 42),
        })
    };
    assert_eq!(unique(&lookup, &arena, &[reference(a)]), a);
    assert_eq!(
        unique(&lookup, &arena, &[reference(a), reference(a)]),
        Lifetime::Unknown
    );
    assert_eq!(
        unique(&lookup, &arena, &[reference(a), reference(b)]),
        Lifetime::Unknown
    );
    assert_eq!(
        unique(
            &lookup,
            &arena,
            &[reference(a), reference(Lifetime::Unknown)]
        ),
        Lifetime::Unknown
    );
    assert_eq!(
        unique(
            &lookup,
            &arena,
            &[reference(a), arena.intern(Type::Unknown)]
        ),
        Lifetime::Unknown
    );
    assert_eq!(
        unique(&lookup, &arena, &[reference(Lifetime::Static)]),
        Lifetime::Static
    );
    assert_eq!(unique(&lookup, &arena, &[]), Lifetime::Unknown);
    assert_eq!(
        unique(&lookup, &arena, &[arena.decl("Doc", 42)]),
        Lifetime::Unknown
    );
    let nested = arena.intern(Type::Function {
        params: vec![reference(a)],
        return_: reference(a),
    });
    assert_eq!(unique(&lookup, &arena, &[nested]), Lifetime::Unknown);
    let repeated = arena.intern(Type::Tuple(vec![reference(a), reference(a)]));
    assert_eq!(
        unique(&lookup, &arena, &[repeated]),
        a,
        "within one parameter, repeated region IDs agree"
    );
    assert_eq!(
        unique(&lookup, &arena, &[repeated, reference(a)]),
        Lifetime::Unknown,
        "parameter boundaries cannot be flattened"
    );
    assert_eq!(
        unique(&Lookup::new(), &arena, &[reference(a)]),
        Lifetime::Unknown,
        "deleted referents are not complete input evidence"
    );
}
