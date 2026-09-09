use super::*;

#[test]
fn source_region_recipes_rebind_parameter_ids_and_fail_closed_on_kind_changes_or_deletion() {
    use crate::type_checker::core::types::{
        GenericParamData, GenericParamKind, Lifetime, Mutability,
    };
    let arena = TypeArena::new();
    let nominal = arena.decl("Doc", 71);
    let p = arena.intern_generic(GenericParamData {
        name: "'a".into(),
        kind: GenericParamKind::Lifetime,
        owner_symbol_index: 1,
        bound: None,
    });
    let q = arena.intern_generic(GenericParamData {
        name: "'a".into(),
        kind: GenericParamKind::Lifetime,
        owner_symbol_index: 2,
        bound: None,
    });
    let recipe = Recipe::Indirect {
        kind: Indirection::Reference(Lifetime::Unknown),
        mutability: Mutability::Shared,
        region: Some(Box::new(Recipe::SourceParameter {
            binding: 8,
            index: 0,
        })),
        inner: Box::new(Recipe::Fixed(nominal)),
    };
    let recipe: Recipe = serde_json::from_str(&serde_json::to_string(&recipe).unwrap()).unwrap();
    for (target, expected) in [
        (Some(arena.generic_type(p)), Lifetime::Parameter(p)),
        (Some(arena.generic_type(q)), Lifetime::Parameter(q)),
        (Some(nominal), Lifetime::Unknown),
        (None, Lifetime::Unknown),
    ] {
        let ty = recipe.materialize_with_parameters(&arena, &|_| None, &|binding, index| {
            assert_eq!((binding, index), (BindingId(8), 0));
            target
        });
        assert_eq!(
            arena.get(ty),
            Type::Indirect {
                kind: Indirection::Reference(expected),
                mutability: Mutability::Shared,
                inner: nominal
            }
        );
    }
}

#[test]
fn retained_indirection_recipe_retargets_child_ids_and_preserves_deletion_uncertainty() {
    use crate::type_checker::core::types::{Indirection, Lifetime, Mutability};
    let arena = TypeArena::new();
    let kind = Indirection::Reference(Lifetime::Static);
    let recipe = Recipe::Indirect {
        kind,
        mutability: Mutability::Mutable,
        region: None,
        inner: Box::new(Recipe::Import(7)),
    };
    let recipe: Recipe = serde_json::from_str(&serde_json::to_string(&recipe).unwrap()).unwrap();
    for id in [Some(71), Some(72), None] {
        let target = id.map(|id| arena.decl("Same", id));
        assert_eq!(
            arena.get(recipe.materialize(&arena, &|binding| {
                assert_eq!(binding, BindingId(7));
                target
            })),
            Type::Indirect {
                kind,
                mutability: Mutability::Mutable,
                inner: target.unwrap_or_else(|| arena.intern(Type::Unknown))
            }
        );
    }
}

#[test]
fn source_recipe_distinguishes_unconfigured_boundaries_from_negative_evidence() {
    let arena = TypeArena::new();
    let legacy = arena.intern_type_str("OldDisplay");
    let unknown = arena.intern(Type::Unknown);
    let recipe = Recipe::Source {
        binding: 8,
        local: false,
        legacy: Some(legacy),
    };
    let recipe: Recipe = serde_json::from_str(&serde_json::to_string(&recipe).unwrap()).unwrap();
    assert_eq!(recipe.materialize(&arena, &|_| None), legacy);
    assert_eq!(recipe.materialize(&arena, &|_| Some(unknown)), unknown);
    for id in [71, 72] {
        let bound = arena.decl("Same", id);
        assert_eq!(recipe.materialize(&arena, &|_| Some(bound)), bound);
    }
    let local = Recipe::Source {
        binding: 8,
        local: true,
        legacy: Some(legacy),
    };
    assert_eq!(local.materialize(&arena, &|_| None), unknown);
}

#[test]
fn retained_structural_recipe_rebinds_import_ids_without_text_or_stale_targets() {
    let arena = TypeArena::new();
    let recipe = Recipe::Function(
        vec![Recipe::Optional(Box::new(Recipe::Import(3)))],
        Box::new(Recipe::Import(3)),
    );
    let recipe: Recipe = serde_json::from_str(&serde_json::to_string(&recipe).unwrap()).unwrap();
    for declaration in [Some(71), Some(72), None] {
        let ty = declaration.map(|id| arena.decl("Same", id));
        let expected = ty.unwrap_or_else(|| arena.intern(Type::Unknown));
        assert_eq!(
            arena.get(recipe.materialize(&arena, &|binding| {
                assert_eq!(binding, BindingId(3));
                ty
            })),
            Type::Function {
                params: vec![arena.intern(Type::Optional(expected))],
                return_: expected
            }
        );
    }
}
