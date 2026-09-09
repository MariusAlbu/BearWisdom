use super::*;

#[test]
fn configured_parent_edges_preserve_exact_arguments_and_fence_cycles_or_conflicts() {
    let arena = TypeArena::new();
    let context = crate::type_checker::core::types::NominalContextId::fresh();
    let parent = arena.decl_in(context, "poisoned", 71);
    let child = arena.decl_in(context, "poisoned", 72);
    let arg = arena.primitive(PrimKind::Bool);
    let applied = arena.intern(Type::Apply {
        base: parent,
        args: vec![arg],
    });
    let mut info = FxHashMap::default();
    let edges = Edges::install(vec![(72, applied)], &mut info, &arena);
    assert_eq!(edges.parent(72), Some(71));
    assert_eq!(edges.args(72, 71), &[arg]);
    assert!(edges.args(72, 99).is_empty());
    assert_eq!(info[&72].base_type_id, Some(applied));
    for pending in [
        vec![(71, child), (72, applied)],
        vec![(72, applied), (72, parent)],
        vec![(72, child)],
    ] {
        let edges = Edges::install(pending, &mut info, &arena);
        assert_eq!(edges.parent(72), None);
        assert_eq!(arena.get(info[&72].base_type_id.unwrap()), Type::Unknown);
    }
}

#[test]
fn source_base_recipes_roundtrip_owner_indices_without_arena_values() {
    let input = Input {
        owner: 72,
        head: Head::Import(3),
        args: vec![Recipe::Parameter {
            owner: 72,
            index: 0,
        }],
    };
    let json = serde_json::to_value(input).unwrap();
    let restored: Input = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), json);
}
