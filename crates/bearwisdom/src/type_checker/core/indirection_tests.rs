use super::super::{Type, TypeArena};
use super::*;

#[test]
fn parameter_kinds_and_named_regions_survive_snapshots_without_spelling_identity() {
    let arena = TypeArena::new();
    let parameter = |kind, owner| {
        arena.intern_generic(super::super::GenericParamData {
            name: "'a".into(),
            kind,
            owner_symbol_index: owner,
            bound: None,
        })
    };
    let a = parameter(GenericParamKind::Lifetime, 1);
    let b = parameter(GenericParamKind::Lifetime, 2);
    let t = parameter(GenericParamKind::Type, 1);
    let c = parameter(GenericParamKind::Const, 1);
    assert_ne!(arena.generic_type(a), arena.generic_type(b));
    assert_ne!(arena.generic_type(a), arena.generic_type(t));
    assert_eq!(arena.get(arena.generic_type(c)), Type::Unknown);
    let ty = arena.intern(Type::Indirect {
        kind: Indirection::Reference(Lifetime::Parameter(a)),
        mutability: Mutability::Shared,
        inner: arena.generic_type(t),
    });
    assert_eq!(arena.format_type(ty), "&'a 'a");
    let restored = TypeArena::new();
    restored.restore_snapshot(&arena.serialize_snapshot());
    assert_eq!(restored.get(ty), arena.get(ty));
    for param in [a, b, t, c] {
        assert_eq!(restored.generic_param(param), arena.generic_param(param));
    }
}

#[test]
fn structural_identity_and_snapshots_preserve_every_indirection_axis() {
    let arena = TypeArena::new();
    let a = arena.decl("Same", 41);
    let b = arena.decl("Same", 42);
    let mut ids = std::collections::HashSet::new();
    for kind in [
        Indirection::Reference(Lifetime::Static),
        Indirection::Reference(Lifetime::Unknown),
        Indirection::Pointer,
    ] {
        for mutability in [Mutability::Shared, Mutability::Mutable] {
            for inner in [a, b] {
                let ty = Type::Indirect {
                    kind,
                    mutability,
                    inner,
                };
                let id = arena.intern(ty.clone());
                assert!(ids.insert(id));
                assert_eq!(arena.intern(ty), id);
            }
        }
    }
    let restored = TypeArena::new();
    assert_eq!(
        restored.restore_snapshot(&arena.serialize_snapshot()),
        arena.len()
    );
    for id in ids {
        assert_eq!(restored.get(id), arena.get(id));
    }
    let ty = arena.intern(Type::Indirect {
        kind: Indirection::Reference(Lifetime::Static),
        mutability: Mutability::Mutable,
        inner: a,
    });
    assert_eq!(arena.format_type(ty), "&'static mut Same");
}
