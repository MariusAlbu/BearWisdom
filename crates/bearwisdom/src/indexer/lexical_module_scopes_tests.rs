use super::*;

fn graph(source: &str) -> LexicalBindings {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    crate::indexer::lexical::capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap()
}

#[test]
fn unit_ids_preserve_source_parents_kinds_and_global_augmentation_provenance() {
    let source = "declare module 'pro\\u0076ider' { namespace Nested { type Alias = string; } global { interface Catalog {} } } declare namespace Other {}";
    let graph = graph(source);
    let units = &graph.module.units;
    assert_eq!(units.len(), 4);
    assert_eq!(
        units.iter().map(|u| u.id).collect::<Vec<_>>(),
        (1..=4).map(SourceModuleId).collect::<Vec<_>>()
    );
    assert_eq!(
        units.iter().map(|u| u.parent.0).collect::<Vec<_>>(),
        [0, 1, 1, 0]
    );
    assert_eq!(
        units.iter().map(|u| u.kind).collect::<Vec<_>>(),
        [
            Kind::Literal,
            Kind::Namespace,
            Kind::Augmentation,
            Kind::Namespace
        ]
    );
    assert_eq!(units[0].name, graph.name_id("provider"));
    assert!(units.iter().all(|u| u.complete && u.ambient));
    for unit in units {
        assert!(unit.range.start < unit.body.start && unit.body.end <= unit.range.end);
        assert_eq!(graph.scope_at(unit.body.start), Some(unit.scope));
    }
    let globals = graph.globals.as_ref().unwrap();
    assert!(
        globals.complete,
        "source ownership is now represented; configured legality is checked separately"
    );
    assert_eq!(globals.augmentations.len(), 1);
    assert_eq!(globals.augmentations[0].unit, units[2].id);
}

#[test]
fn augmentation_vars_do_not_acquire_the_enclosing_module_local_binding() {
    let source = "declare module 'provider' { var item: string; global { var item: number; type Inside = typeof item; } type Outside = typeof item; } item;";
    let graph = graph(source);
    let name = graph.name_id("item").unwrap();
    let uses: Vec<_> = source
        .match_indices("typeof item")
        .map(|(byte, _)| graph.binding_at(byte as u32, name).unwrap())
        .collect();
    assert_ne!(uses[0], uses[1]);
    assert_eq!(
        graph.binding_at(source.rfind("item;").unwrap() as u32, name),
        None
    );
}

#[test]
fn ambient_export_sets_follow_declaration_modifiers_not_export_declarations() {
    let fixtures: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../tree-sitter-typescript-local/module-scope-fixtures.json"
    ))
    .unwrap();
    for fixture in fixtures {
        let source = fixture["source"].as_str().unwrap();
        let graph = graph(source);
        let expected: Vec<Vec<String>> =
            serde_json::from_value(fixture["exports"].clone()).unwrap();
        assert_eq!(graph.module.units.len(), expected.len(), "{source}");
        for (unit, mut expected) in graph.module.units.iter().zip(expected) {
            assert!(unit.complete, "{source}: {unit:?}");
            let mut actual: Vec<_> = unit.exports.iter().map(|e| e.name.clone()).collect();
            actual.sort();
            expected.sort();
            assert_eq!(actual, expected, "{source}");
        }
        if source.starts_with("declare module") {
            assert!(
                graph.module.exports.is_empty(),
                "ambient providers are not file exports"
            );
        }
    }
}

#[test]
fn unsupported_dotted_names_and_assignment_expressions_remain_explicitly_incomplete() {
    for source in [
        "declare namespace Outer.Inner {}",
        "declare module 'provider' { function make(): void; export = make(); }",
    ] {
        let graph = graph(source);
        assert!(
            !graph.module.units[0].complete,
            "{source}: {:?}",
            graph.module.units
        );
    }
}

#[test]
fn detached_import_selectors_retain_unit_ownership_and_duplicate_barriers() {
    let source = "declare module 'provider' { import * as NS from 'left'; import * as NS from 'right'; type Item = NS.Doc; }";
    let graph = graph(source);
    assert!(
        graph.module.imports.len() >= 2,
        "namespace selector must be captured separately"
    );
    assert_eq!(
        graph.module.imports.len(),
        graph.module.ambiguous_imports.len()
    );
    for binding in graph.module.imports.keys() {
        assert_eq!(graph.module.import_units[binding], SourceModuleId(1));
    }
}

#[test]
fn unrepresentable_literal_names_never_fall_back_to_local_exports() {
    for source in [
        "const Known = 1; export { Known as '\\uD800' };",
        "const Known = 1; export { Known } from '\\uD800';",
        "import { '\\uD800' as Known } from 'provider'; export { Known };",
    ] {
        let graph = graph(source);
        assert!(!graph.module.complete, "{source}");
    }
}
