use super::*;

#[test]
fn grouped_renames_keep_original_path_and_anonymous_import_identity() {
    let source = b"use model::{self as models, Doc as Alias, Hidden as _};";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let forms = syntax_for("rust").unwrap();
    let mut data = NamespaceData::default();
    let mut uses = Vec::new();
    imports(
        tree.root_node()
            .named_child(0)
            .unwrap()
            .child_by_field_name("argument")
            .unwrap(),
        &[],
        source,
        forms,
        &mut data,
        &mut uses,
    );
    assert_eq!(uses.len(), 3);
    assert_eq!(data.spelling(uses[0].name), "models");
    assert_eq!(
        uses[0]
            .path
            .iter()
            .map(|(name, _)| data.spelling(*name))
            .collect::<Vec<_>>(),
        vec!["model"]
    );
    assert_eq!(
        uses[1]
            .path
            .iter()
            .map(|(name, _)| data.spelling(*name))
            .collect::<Vec<_>>(),
        vec!["model", "Doc"]
    );
    assert!(uses[2].anonymous);
    assert!(!uses[0].anonymous && !uses[1].anonymous);
    assert_eq!(
        uses[2]
            .path
            .iter()
            .map(|(name, _)| data.spelling(*name))
            .collect::<Vec<_>>(),
        vec!["model", "Hidden"]
    );
}

#[test]
fn root_and_restriction_recipes_keep_context_and_do_not_consult_lexical_names() {
    let forms = syntax_for("rust").unwrap();
    let mut data = NamespaceData::default();
    let scope = data.graph.add_scope(None, 0, 100, false);
    let origin = SourceModuleId(7);
    data.scope_units.insert(scope, origin);
    let names: Vec<_> = ["self", "super", "crate", "outer"]
        .iter()
        .map(|name| data.intern(name, forms))
        .collect();
    assert!(
        matches!(target(&data, forms, scope, &[(names[0], 0), (names[3], 6)], ExportDomain::Type),
        Target::Select(base, _, owner) if matches!(*base, Target::Module(id) if id == origin) && owner == origin)
    );
    assert!(
        matches!(rooted(&data, origin, &[(names[1], 0), (names[1], 7), (names[3], 14)]),
        Some((Target::Parent(id, 2), 2)) if id == origin)
    );
    assert!(
        matches!(restriction(&data, origin, &[(names[2], 0), (names[3], 7)]),
        Target::DeclarationPath(base, selectors) if matches!(*base, Target::CrateRoot) && selectors == vec![names[3]])
    );
    assert!(matches!(
        restriction(&data, origin, &[(names[3], 0)]),
        Target::Missing
    ));
    assert!(matches!(
        restriction(&data, origin, &[(names[2], 0), (names[0], 7)]),
        Target::Missing
    ));
}
