use super::*;

#[test]
fn overloaded_callable_yields_keep_all_return_ids_and_unknown_branches() {
    let arena = TypeArena::new();
    let left = arena.decl("Same", 71);
    let right = arena.decl("Same", 72);
    let signature = |ty| {
        arena.intern(Type::Function {
            params: vec![],
            return_: ty,
        })
    };
    let group = arena.intern(Type::Intersection(vec![signature(left), signature(right)]));
    let result = callable_return(&arena, group).unwrap();
    assert_eq!(arena.get(result), Type::Union(vec![left, right]));
    let unknown = arena.intern(Type::Unknown);
    let group = arena.intern(Type::Intersection(vec![
        signature(left),
        signature(unknown),
    ]));
    assert_eq!(callable_return(&arena, group), Some(unknown));
    assert_eq!(
        callable_return(&arena, arena.intern(Type::Intersection(vec![left, right]))),
        None
    );
}
use crate::indexer::resolve::engine::testkit::Lookup;

#[test]
fn constructor_arguments_preserve_distinct_declarations_with_identical_display() {
    let arena = TypeArena::new();
    let class = arena.decl("Same", 71);
    let argument = arena.decl("Same", 72);
    let local = LocalReference {
        declaration: Some(3),
        kind: SymbolKind::Variable,
        value_type: Some(arena.intern(Type::Constructor(class))),
        callable: None,
        type_args: vec![argument],
    };
    let ty = yield_type(&local, &Lookup::new(), &arena, true).unwrap();
    assert_eq!(
        arena.get(ty),
        Type::Apply {
            base: class,
            args: vec![argument]
        }
    );
    assert_eq!(yield_type(&local, &Lookup::new(), &arena, false), None);
}

#[test]
fn explicit_callable_contract_is_not_replaced_by_provenance() {
    let arena = TypeArena::new();
    let result = arena.intern(Type::Optional(arena.decl("Same", 71)));
    let local = LocalReference {
        declaration: Some(3),
        kind: SymbolKind::Parameter,
        value_type: Some(arena.intern(Type::Function {
            params: vec![],
            return_: result,
        })),
        callable: Some(404),
        type_args: vec![],
    };
    assert_eq!(
        yield_type(&local, &Lookup::new(), &arena, false),
        Some(result)
    );
    assert_eq!(yield_type(&local, &Lookup::new(), &arena, true), None);
}
