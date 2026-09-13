use super::*;

fn unit(id: u32, name: &str) -> InputUnit {
    serde_json::from_value(serde_json::json!({
        "id": id, "parent": 0, "source_name": name,
        "exports": [], "imports": [], "stars": [], "wildcard_exclusions": [],
        "source_scope": { "kind": "Literal", "lexical_scope": 1,
            "range": { "start": 0, "end": 40 }, "body": { "start": 12, "end": 40 },
            "ambient": true, "container_valid": true, "complete": true }
    }))
    .unwrap()
}

fn source(path: &str, isolated: bool, units: Vec<InputUnit>) -> ModuleInput {
    ModuleInput {
        path: path.into(),
        units,
        globals: Some(super::super::super::program_input::Input {
            isolated,
            complete: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn inputs(sources: Vec<ModuleInput>) -> BTreeMap<String, ModuleInput> {
    sources
        .into_iter()
        .map(|input| (input.path.clone(), input))
        .collect()
}

#[test]
fn only_a_top_level_complete_container_valid_ambient_named_literal_is_admitted() {
    assert!(is_literal(&unit(1, "path")));
    assert_eq!(admitted_specifier(&unit(1, "path")), Some("path"));

    let mut nested = unit(1, "path");
    nested.parent = SourceModuleId(2);
    assert!(
        is_literal(&nested),
        "a nested unit is still literal evidence"
    );
    assert_eq!(admitted_specifier(&nested), None);

    let mut nameless = unit(1, "path");
    nameless.source_name = None;
    assert_eq!(admitted_specifier(&nameless), None);

    let mut ambientless = unit(1, "path");
    ambientless.source_scope.as_mut().unwrap().ambient = false;
    assert_eq!(admitted_specifier(&ambientless), None);

    let mut partial = unit(1, "path");
    partial.source_scope.as_mut().unwrap().complete = false;
    assert_eq!(admitted_specifier(&partial), None);

    let mut misplaced = unit(1, "path");
    misplaced.source_scope.as_mut().unwrap().container_valid = false;
    assert_eq!(admitted_specifier(&misplaced), None);

    let mut namespaced = unit(1, "path");
    namespaced.source_scope.as_mut().unwrap().kind = Kind::Namespace;
    assert!(!is_literal(&namespaced));
    assert_eq!(admitted_specifier(&namespaced), None);
}

#[test]
fn a_non_isolated_declaration_provides_its_module_and_an_isolated_one_never_does() {
    let inputs = inputs(vec![
        source(
            "types.d.ts",
            false,
            vec![unit(1, "path"), unit(2, "node:path")],
        ),
        source("app.ts", true, vec![unit(1, "react")]),
    ]);
    let mut providers = BTreeMap::new();
    seed_providers(&mut providers, &inputs);
    assert_eq!(
        providers.keys().collect::<Vec<_>>(),
        vec!["node:path", "path"],
        "an augmentation never manufactures the module it augments"
    );
    assert_eq!(
        providers["path"].parts,
        vec![("types.d.ts".to_owned(), SourceModuleId(1))]
    );
}

#[test]
fn a_relative_name_is_skipped_and_a_pre_seeded_entry_survives_repeated_seeding() {
    let inputs = inputs(vec![source(
        "pkg/types.d.ts",
        false,
        vec![unit(1, "./local"), unit(2, "path")],
    )]);
    let mut providers = BTreeMap::new();
    providers.insert(
        "path".to_owned(),
        program_modules::Provider {
            parts: vec![("selected.d.ts".to_owned(), SourceModuleId(9))],
        },
    );
    seed_providers(&mut providers, &inputs);
    seed_providers(&mut providers, &inputs);
    assert_eq!(
        providers.keys().collect::<Vec<_>>(),
        vec!["path"],
        "a relative ambient name is not a globally keyed provider"
    );
    assert_eq!(
        providers["path"].parts,
        vec![("selected.d.ts".to_owned(), SourceModuleId(9))],
        "the caller's selection stays authoritative"
    );
}
