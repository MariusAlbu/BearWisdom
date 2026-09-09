use super::super::{Indirection, Lifetime, Mutability};
use super::*;

#[test]
fn rebind_preserves_indirection_and_never_rebinds_a_declaration_by_display() {
    let arena = TypeArena::new();
    let param = arena.intern_generic(super::super::GenericParamData {
        kind: Default::default(),
        name: "T".into(),
        owner_symbol_index: 0,
        bound: None,
    });
    let replacement = arena.intern(Type::Generic { param });
    let names = FxHashMap::from_iter([("T".into(), replacement)]);
    for inner in [arena.class("T"), arena.decl("T", 41)] {
        let ty = Type::Indirect {
            kind: Indirection::Reference(Lifetime::Static),
            mutability: Mutability::Mutable,
            inner,
        };
        let got = arena.rebind_class_params(arena.intern(ty.clone()), &names);
        let expected = if matches!(arena.get(inner), Type::Class(_)) {
            replacement
        } else {
            inner
        };
        assert_eq!(
            arena.get(got),
            Type::Indirect {
                kind: Indirection::Reference(Lifetime::Static),
                mutability: Mutability::Mutable,
                inner: expected
            }
        );
    }
}
