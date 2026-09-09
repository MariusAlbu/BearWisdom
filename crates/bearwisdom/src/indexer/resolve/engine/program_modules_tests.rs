use super::super::super::{
    module_input::{InputBinding, InputExport, InputUnit},
    testkit::{sym, Lookup},
};
use super::*;

fn source(path: &str) -> ModuleInput {
    ModuleInput {
        path: path.into(),
        content_hash: "source".into(),
        binding_epoch: super::super::super::module_input::BINDING_EPOCH,
        globals: Some(super::super::super::program_input::Input {
            complete: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn configured_fixture() -> (ModuleGraph, Lookup) {
    use crate::indexer::programs::{Program, ProgramSource, SourceScope};
    let mut graph = ModuleGraph::default();
    let mut lookup = Lookup::new();
    for (path, row) in [("a.d.ts", 71), ("b.d.ts", 72), ("outside.d.ts", 73)] {
        let mut input = source(path);
        let mut unit: InputUnit = serde_json::from_value(serde_json::json!({ "id": 1, "parent": 0, "source_name": "provider",
            "exports": [], "imports": [], "stars": [], "wildcard_exclusions": [],
            "source_scope": { "kind": "Literal", "lexical_scope": 1, "range": { "start": 0, "end": 40 },
                "body": { "start": 12, "end": 40 }, "ambient": true, "container_valid": true, "complete": true }
        })).unwrap();
        unit.exports.push(InputExport {
            name: "make".into(),
            domain: ExportDomain::Value,
            target: InputTarget::Declaration(row),
        });
        input.units.push(unit);
        graph.inputs.insert(path.into(), input);
        lookup = lookup.with(sym(row, "poisoned", "poisoned", "function", path));
    }
    let mut caller = source("shared.ts");
    caller.globals.as_mut().unwrap().isolated = true;
    caller.imports.push(InputBinding {
        binding: 1,
        domain: ExportDomain::Value,
        target: InputTarget::From {
            module: "provider".into(),
            name: "make".into(),
        },
    });
    graph.inputs.insert(caller.path.clone(), caller);
    graph.programs.configuration = Some(
        [("a", "a.d.ts"), ("b", "b.d.ts")]
            .map(|(key, provider)| Program {
                key: key.into(),
                fingerprint: key.into(),
                complete: true,
                callable_policy: None,
                compiler_intrinsics: None,
                source_binding_order: None,
                sources: ["shared.ts", provider]
                    .map(|path| ProgramSource {
                        path: path.into(),
                        content_hash: "source".into(),
                        scope: SourceScope::Syntax,
                    })
                    .to_vec(),
            })
            .to_vec(),
    );
    graph.rebuild(&lookup);
    (graph, lookup)
}

#[test]
fn same_physical_consumer_gets_different_provider_ids_in_overlapping_programs() {
    let (graph, lookup) = configured_fixture();
    assert_eq!(
        graph.binding("shared.ts", BindingId(1), false),
        BindingResult::Missing
    );
    for (key, row) in [("a", 71), ("b", 72)] {
        let selected = graph
            .for_program(graph.programs.program(key).unwrap(), &lookup)
            .unwrap();
        assert_eq!(
            selected.binding("shared.ts", BindingId(1), false),
            BindingResult::Bound(row)
        );
        assert!(!selected.inputs.contains_key("outside.d.ts"));
    }
}

#[test]
fn assigned_entities_keep_overlapping_program_facets_separate_after_reload() {
    let (mut graph, lookup) = configured_fixture();
    for (path, row) in [("a.d.ts", 71), ("b.d.ts", 72), ("outside.d.ts", 73)] {
        graph.inputs.get_mut(path).unwrap().units[0]
            .assignments
            .push((
                InputTarget::Entity {
                    declaration: Box::new(InputTarget::Declaration(row)),
                    namespace: Box::new(InputTarget::LocalNamespace(SourceModuleId(1))),
                },
                ExportDomain::Value,
            ));
    }
    let caller = graph.inputs.get_mut("shared.ts").unwrap();
    caller.imports[0].target = InputTarget::Assigned {
        module: "provider".into(),
    };
    caller.imports.push(InputBinding {
        binding: 2,
        domain: ExportDomain::Value,
        target: InputTarget::Path {
            base: Box::new(InputTarget::Binding {
                module: SourceModuleId(0),
                binding: 1,
                domain: ExportDomain::Value,
            }),
            names: vec!["make".into()],
        },
    });
    graph.rebuild(&lookup);
    let check = |graph: &ModuleGraph| {
        assert_eq!(
            graph.binding("shared.ts", BindingId(1), false),
            BindingResult::Missing
        );
        for (key, row) in [("a", 71), ("b", 72)] {
            let selected = graph
                .for_program(graph.programs.program(key).unwrap(), &lookup)
                .unwrap();
            let value = selected.binding("shared.ts", BindingId(1), false);
            assert_eq!(value.declaration(), Some(row));
            assert!(value.namespace().is_some());
            assert_eq!(
                selected.binding("shared.ts", BindingId(2), false),
                BindingResult::Bound(row)
            );
        }
    };
    check(&graph);
    let db = crate::Database::open_in_memory().unwrap();
    for path in graph.inputs.keys() {
        db.conn().execute("INSERT INTO files (path,hash,language,last_indexed) VALUES (?1,'source','typescript',0)", [path]).unwrap();
    }
    graph.persist(db.conn()).unwrap();
    let mut cold = ModuleGraph::default();
    cold.load(db.conn()).unwrap();
    cold.rebuild(&lookup);
    check(&cold);
    db.conn()
        .execute("DELETE FROM files WHERE path='a.d.ts'", [])
        .unwrap();
    let mut deleted = ModuleGraph::default();
    deleted.load(db.conn()).unwrap();
    deleted.rebuild(&lookup);
    assert!(deleted
        .for_program(deleted.programs.program("a").unwrap(), &lookup)
        .is_none());
    let selected = deleted
        .for_program(deleted.programs.program("b").unwrap(), &lookup)
        .unwrap();
    assert_eq!(
        selected
            .binding("shared.ts", BindingId(1), false)
            .declaration(),
        Some(72)
    );
    assert_eq!(
        selected.binding("shared.ts", BindingId(2), false),
        BindingResult::Bound(72)
    );
}

#[test]
fn program_provider_selection_rebuilds_from_persisted_source_fingerprints() {
    let (graph, lookup) = configured_fixture();
    let db = crate::Database::open_in_memory().unwrap();
    for path in graph.inputs.keys() {
        db.conn().execute("INSERT INTO files (path,hash,language,last_indexed) VALUES (?1,'source','typescript',0)", [path]).unwrap();
    }
    graph.persist(db.conn()).unwrap();
    let mut cold = ModuleGraph::default();
    cold.load(db.conn()).unwrap();
    cold.rebuild(&lookup);
    for (key, row) in [("a", 71), ("b", 72)] {
        let selected = cold
            .for_program(cold.programs.program(key).unwrap(), &lookup)
            .unwrap();
        assert_eq!(
            selected.binding("shared.ts", BindingId(1), false),
            BindingResult::Bound(row)
        );
    }
    cold.inputs.get_mut("a.d.ts").unwrap().units[0].source_name = Some("renamed".into());
    cold.rebuild(&lookup);
    let selected = cold
        .for_program(cold.programs.program("a").unwrap(), &lookup)
        .unwrap();
    assert_eq!(
        selected.binding("shared.ts", BindingId(1), false),
        BindingResult::Missing
    );
    for statement in [
        "UPDATE files SET hash='stale' WHERE path='a.d.ts'",
        "DELETE FROM files WHERE path='a.d.ts'",
    ] {
        db.conn().execute(statement, []).unwrap();
        let mut cold = ModuleGraph::default();
        cold.load(db.conn()).unwrap();
        cold.rebuild(&lookup);
        assert!(cold
            .for_program(cold.programs.program("a").unwrap(), &lookup)
            .is_none());
        let selected = cold
            .for_program(cold.programs.program("b").unwrap(), &lookup)
            .unwrap();
        assert_eq!(
            selected.binding("shared.ts", BindingId(1), false),
            BindingResult::Bound(72)
        );
    }
}

#[test]
fn program_provider_groups_preserve_competing_targets_and_missing_parts() {
    let mut graph = ModuleGraph::default();
    let input = ModuleInput {
        path: "provider.ts".into(),
        units: (1..=2)
            .map(|id| InputUnit {
                id: SourceModuleId(id),
                exports: vec![InputExport {
                    name: "make".into(),
                    domain: ExportDomain::Value,
                    target: InputTarget::Declaration(id as i64 + 70),
                }],
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    graph.inputs.insert(input.path.clone(), input);
    graph.inputs.insert(
        "main.ts".into(),
        ModuleInput {
            path: "main.ts".into(),
            imports: vec![InputBinding {
                binding: 1,
                domain: ExportDomain::Value,
                target: InputTarget::From {
                    module: "provider".into(),
                    name: "make".into(),
                },
            }],
            ..Default::default()
        },
    );
    graph.providers.insert(
        "provider".into(),
        Provider {
            parts: vec![("provider.ts".into(), SourceModuleId(1))],
        },
    );
    let lookup = Lookup::new()
        .with(sym(71, "poisoned", "poisoned", "function", "provider.ts"))
        .with(sym(72, "poisoned", "poisoned", "function", "provider.ts"));
    graph.rebuild(&lookup);
    assert_eq!(
        graph.binding("main.ts", BindingId(1), false),
        BindingResult::Bound(71)
    );
    graph
        .providers
        .get_mut("provider")
        .unwrap()
        .parts
        .push(("provider.ts".into(), SourceModuleId(2)));
    graph.rebuild(&lookup);
    assert_eq!(
        graph.binding("main.ts", BindingId(1), false),
        BindingResult::Ambiguous
    );
    graph
        .providers
        .get_mut("provider")
        .unwrap()
        .parts
        .push(("absent.ts".into(), SourceModuleId(1)));
    graph.rebuild(&lookup);
    assert_eq!(
        graph.binding("main.ts", BindingId(1), false),
        BindingResult::Incomplete
    );
    graph.providers.clear();
    graph.rebuild(&lookup);
    assert_eq!(
        graph.binding("main.ts", BindingId(1), false),
        BindingResult::Missing,
        "workspace graphs do not infer literal providers"
    );
}

#[test]
fn physical_module_augmentation_follows_the_target_id_not_a_shared_relative_specifier() {
    let mut graph = ModuleGraph::default();
    for path in ["a/model.ts", "b/model.ts"] {
        graph.inputs.insert(
            path.into(),
            ModuleInput {
                path: path.into(),
                ..Default::default()
            },
        );
    }
    graph.inputs.insert(
        "augmentation.ts".into(),
        ModuleInput {
            path: "augmentation.ts".into(),
            units: vec![InputUnit {
                id: SourceModuleId(1),
                exports: vec![InputExport {
                    name: "extra".into(),
                    domain: ExportDomain::Value,
                    target: InputTarget::Declaration(71),
                }],
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    for (path, spec) in [
        ("a/main.ts", "./model.ts"),
        ("b/main.ts", "./model.ts"),
        ("root.ts", "./a/model.ts"),
    ] {
        graph.inputs.insert(
            path.into(),
            ModuleInput {
                path: path.into(),
                imports: vec![InputBinding {
                    binding: 1,
                    domain: ExportDomain::Value,
                    target: InputTarget::From {
                        module: spec.into(),
                        name: "extra".into(),
                    },
                }],
                ..Default::default()
            },
        );
    }
    graph.augmentations.insert(
        "a/model.ts".into(),
        Provider {
            parts: vec![
                ("a/model.ts".into(), SourceModuleId(0)),
                ("augmentation.ts".into(), SourceModuleId(1)),
            ],
        },
    );
    graph.rebuild(&Lookup::new().with(sym(
        71,
        "poisoned",
        "poisoned",
        "function",
        "augmentation.ts",
    )));
    for path in ["a/main.ts", "root.ts"] {
        assert_eq!(
            graph.binding(path, BindingId(1), false),
            BindingResult::Bound(71)
        );
    }
    assert_eq!(
        graph.binding("b/main.ts", BindingId(1), false),
        BindingResult::Missing
    );
}
