use super::*;

#[test]
fn persisted_program_recipes_preserve_source_names_and_physical_parameter_owners() {
    let mut graph = crate::indexer::lexical::LexicalBindings::default();
    let name = graph.intern("Catalog");
    let recipe = Recipe::Function(
        vec![Recipe::Parameter {
            owner: 42,
            index: 1,
        }],
        Box::new(Recipe::Apply(
            Box::new(Recipe::Global(name)),
            vec![Recipe::Declaration(vec![17, 23])],
        )),
    );
    let payload = serde_json::to_value(&recipe).unwrap();
    let restored: Recipe = serde_json::from_value(payload.clone()).unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), payload);
}

#[test]
fn program_return_templates_do_not_rebind_display_names() {
    use crate::type_checker::core::types::GenericParamData;
    let arena = TypeArena::new();
    let parameter = arena.intern_generic(GenericParamData {
        name: "T".into(),
        kind: GenericParamKind::Type,
        owner_symbol_index: 1,
        bound: None,
    });
    let ty = arena.class("T");
    let mut info = TypeInfo {
        generic_param_ids: vec![parameter],
        return_type_id: Some(ty),
        ..Default::default()
    };
    finish(&mut info);
    assert_eq!(
        info.generic_return
            .unwrap()
            .instantiate(&arena, &[arena.primitive(PrimKind::Bool)]),
        Some(ty)
    );
}
