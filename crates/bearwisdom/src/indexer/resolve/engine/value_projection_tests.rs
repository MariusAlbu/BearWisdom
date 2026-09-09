use super::*;
use crate::indexer::resolve::engine::testkit::Lookup;
use crate::type_checker::core::types::{Lifetime, Mutability};

#[test]
fn implicit_field_projection_is_reference_only_and_never_uses_display_heads() {
    let lookup = Lookup::new();
    let arena = TypeArena::new();
    let doc = arena.decl("same", 71);
    let reference = arena.intern(Type::Indirect {
        kind: Indirection::Reference(Lifetime::Static),
        mutability: Mutability::Shared,
        inner: doc,
    });
    let nested = arena.intern(Type::Indirect {
        kind: Indirection::Reference(Lifetime::Unknown),
        mutability: Mutability::Mutable,
        inner: reference,
    });
    assert_eq!(field_receiver(&lookup, &arena, nested, true), Some(doc));
    assert_eq!(field_receiver(&lookup, &arena, nested, false), None);
    let raw = arena.intern(Type::Indirect {
        kind: Indirection::Pointer,
        mutability: Mutability::Shared,
        inner: doc,
    });
    assert_eq!(field_receiver(&lookup, &arena, raw, true), None);
    assert_eq!(
        field_receiver(&lookup, &arena, arena.class("same"), true),
        None
    );
    let mut deep = doc;
    for _ in 0..33 {
        deep = arena.intern(Type::Indirect {
            kind: Indirection::Reference(Lifetime::Static),
            mutability: Mutability::Shared,
            inner: deep,
        });
    }
    assert_eq!(field_receiver(&lookup, &arena, deep, true), None);
}
