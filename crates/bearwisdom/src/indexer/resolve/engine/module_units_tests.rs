use super::super::super::{
    module_input::{InputBinding, InputExport, InputUnit, BINDING_EPOCH},
    testkit::{sym, Lookup},
};
use super::*;

#[test]
fn module_parent_order_is_arbitrary_but_cycles_duplicates_and_dangling_ids_are_invalid() {
    let units: Vec<_> = [
        (3, 2),
        (2, 1),
        (1, 0),
        (4, 5),
        (5, 4),
        (6, 4),
        (7, 99),
        (8, 0),
        (8, 1),
        (9, 8),
        (0, 0),
    ]
    .into_iter()
    .map(|(id, parent)| InputUnit {
        id: SourceModuleId(id),
        parent: SourceModuleId(parent),
        ..Default::default()
    })
    .collect();
    assert_eq!(
        valid_units(&units),
        [SourceModuleId(1), SourceModuleId(2), SourceModuleId(3)]
            .into_iter()
            .collect()
    );
}

#[test]
fn incomplete_source_units_cannot_publish_known_exports_or_forwarded_imports() {
    let mut input = model();
    let evidence = serde_json::json!({ "kind": "Namespace", "lexical_scope": 1,
        "range": { "start": 0, "end": 40 }, "body": { "start": 12, "end": 40 },
        "ambient": true, "container_valid": true, "complete": false });
    input.units[0].source_scope = Some(serde_json::from_value(evidence).unwrap());
    input.units[0]
        .imports
        .push(import(9, ExportDomain::Value, InputTarget::Declaration(79)));
    input.imports.push(import(
        9,
        ExportDomain::Value,
        InputTarget::Binding {
            module: SourceModuleId(1),
            binding: 9,
            domain: ExportDomain::Value,
        },
    ));
    let mut graph = ModuleGraph::default();
    graph.inputs.insert(input.path.clone(), input);
    graph.rebuild(&lookup());
    assert_eq!(
        graph.binding("lib.rs", BindingId(0), false),
        BindingResult::Incomplete
    );
    assert_eq!(
        graph.binding("lib.rs", BindingId(9), false),
        BindingResult::Incomplete
    );
    assert_eq!(
        graph.binding_in(
            "lib.rs",
            SourceModuleId(1),
            BindingId(9),
            ExportDomain::Value
        ),
        BindingResult::Incomplete
    );
    assert_eq!(
        graph.binding_in(
            "lib.rs",
            SourceModuleId(2),
            BindingId(0),
            ExportDomain::Value
        ),
        BindingResult::Bound(74),
        "sibling remains independent"
    );
    graph.inputs.get_mut("lib.rs").unwrap().source_complete = Some(false);
    graph.rebuild(&lookup());
    assert_eq!(
        graph.binding("lib.rs", BindingId(9), false),
        BindingResult::Incomplete
    );
}

#[test]
fn deep_module_ancestry_does_not_require_recursive_stack_frames() {
    let units: Vec<_> = (1..=10_000)
        .rev()
        .map(|id| InputUnit {
            id: SourceModuleId(id),
            parent: SourceModuleId(id - 1),
            ..Default::default()
        })
        .collect();
    assert_eq!(valid_units(&units).len(), 10_000);
}

fn export(name: &str, domain: ExportDomain, target: InputTarget) -> InputExport {
    InputExport {
        name: name.into(),
        domain,
        target,
    }
}

fn import(binding: usize, domain: ExportDomain, target: InputTarget) -> InputBinding {
    InputBinding {
        binding,
        domain,
        target,
    }
}

fn from_unit(id: u32, name: &str) -> InputTarget {
    InputTarget::LocalExport {
        module: SourceModuleId(id),
        name: name.into(),
    }
}

fn source(path: &str) -> ModuleInput {
    ModuleInput {
        path: path.into(),
        content_hash: "current".into(),
        binding_epoch: BINDING_EPOCH,
        ..Default::default()
    }
}

fn lookup() -> Lookup {
    // All display names deliberately collide. Only exact declaration IDs attest targets.
    (71..=79).fold(Lookup::new(), |lookup, id| {
        lookup.with(sym(id, "Item", "Item", "function", "lib.rs"))
    })
}

fn model() -> ModuleInput {
    let mut input = source("lib.rs");
    input.units = vec![
        InputUnit {
            id: SourceModuleId(1),
            ..Default::default()
        },
        InputUnit {
            id: SourceModuleId(2),
            ..Default::default()
        },
    ];
    input.exports.push(export(
        "real",
        ExportDomain::Type,
        InputTarget::LocalNamespace(SourceModuleId(1)),
    ));
    for (domain, id) in [
        (ExportDomain::Type, 71),
        (ExportDomain::Value, 72),
        (ExportDomain::Macro, 73),
    ] {
        input.units[0]
            .exports
            .push(export("Item", domain, InputTarget::Declaration(id)));
        input.units[1]
            .exports
            .push(export("Item", domain, InputTarget::Declaration(74)));
        input
            .exports
            .push(export("Alias", domain, from_unit(1, "Item")));
        input.imports.push(import(0, domain, from_unit(1, "Item")));
        input.units[1]
            .imports
            .push(import(0, domain, from_unit(2, "Item")));
    }
    input
}

#[test]
fn nested_module_bindings_and_rename_targets_keep_three_domains_and_source_identity() {
    let mut graph = ModuleGraph::default();
    graph.inputs.insert("lib.rs".into(), model());
    let mut consumer = source("consumer.rs");
    for domain in [ExportDomain::Type, ExportDomain::Value, ExportDomain::Macro] {
        consumer.imports.push(import(
            0,
            domain,
            InputTarget::From {
                module: "./lib.rs".into(),
                name: "Alias".into(),
            },
        ));
        consumer.imports.push(import(
            1,
            domain,
            InputTarget::Select {
                base: Box::new(InputTarget::Namespace {
                    module: "./lib.rs".into(),
                }),
                selectors: vec![("real".into(), ExportDomain::Type), ("Item".into(), domain)],
            },
        ));
    }
    graph.inputs.insert(consumer.path.clone(), consumer);
    let mut other = model();
    other.path = "other.rs".into();
    for export in &mut other.units[0].exports {
        export.target = InputTarget::Declaration(75);
    }
    graph.inputs.insert(other.path.clone(), other);
    graph.rebuild(&lookup());
    for (domain, id) in [
        (ExportDomain::Type, 71),
        (ExportDomain::Value, 72),
        (ExportDomain::Macro, 73),
    ] {
        for binding in [0, 1] {
            assert_eq!(
                graph.binding_in("consumer.rs", SourceModuleId(0), BindingId(binding), domain),
                BindingResult::Bound(id)
            );
        }
        assert_eq!(
            graph.binding_in("lib.rs", SourceModuleId(0), BindingId(0), domain),
            BindingResult::Bound(id)
        );
        assert_eq!(
            graph.binding_in("lib.rs", SourceModuleId(2), BindingId(0), domain),
            BindingResult::Bound(74)
        );
        assert_eq!(
            graph.binding_in("other.rs", SourceModuleId(0), BindingId(0), domain),
            BindingResult::Bound(75)
        );
    }
    let root = graph.paths["lib.rs"];
    let child = graph.units[&(root, SourceModuleId(1))];
    assert_eq!(graph.modules[child.0].parent, Some(root));
    assert_ne!(
        child,
        graph.units[&(graph.paths["other.rs"], SourceModuleId(1))]
    );
}

#[test]
fn malformed_local_module_ids_never_fall_back_to_the_root_or_another_file() {
    let mut input = model();
    input.units.extend(
        [(3, 4), (4, 3), (5, 99), (6, 0), (6, 0), (0, 0)].map(|(id, parent)| InputUnit {
            id: SourceModuleId(id),
            parent: SourceModuleId(parent),
            exports: vec![export(
                "Item",
                ExportDomain::Value,
                InputTarget::Declaration(79),
            )],
            ..Default::default()
        }),
    );
    for unit in 3..=6 {
        input.imports.push(import(
            unit as usize,
            ExportDomain::Value,
            from_unit(unit, "Item"),
        ));
    }
    let mut graph = ModuleGraph::default();
    graph.inputs.insert(input.path.clone(), input);
    graph.rebuild(&lookup());
    for unit in 3..=6 {
        assert_eq!(
            graph.binding("lib.rs", BindingId(unit), false),
            BindingResult::Missing
        );
    }
    assert_eq!(
        graph.binding("lib.rs", BindingId(0), false),
        BindingResult::Bound(72),
        "invalid unit zero cannot overwrite the root"
    );
}

#[test]
fn nested_wildcard_providers_keep_ambiguity_and_domain_policy() {
    let mut input = model();
    let mut barrel = InputUnit {
        id: SourceModuleId(3),
        ..Default::default()
    };
    for domain in [ExportDomain::Type, ExportDomain::Value, ExportDomain::Macro] {
        barrel
            .stars
            .extend([1, 2].map(|id| (InputTarget::LocalNamespace(SourceModuleId(id)), domain)));
        input.imports.push(import(1, domain, from_unit(3, "Item")));
    }
    barrel.exports.push(export(
        "Item",
        ExportDomain::Macro,
        InputTarget::Declaration(73),
    ));
    input.units.push(barrel);
    let mut graph = ModuleGraph::default();
    graph.inputs.insert(input.path.clone(), input);
    graph.rebuild(&lookup());
    for domain in [ExportDomain::Type, ExportDomain::Value] {
        assert_eq!(
            graph.binding_in("lib.rs", SourceModuleId(0), BindingId(1), domain),
            BindingResult::Ambiguous
        );
    }
    assert_eq!(
        graph.binding_in(
            "lib.rs",
            SourceModuleId(0),
            BindingId(1),
            ExportDomain::Macro
        ),
        BindingResult::Bound(73)
    );
    graph.inputs.get_mut("lib.rs").unwrap().units[2]
        .exports
        .clear();
    graph.inputs.get_mut("lib.rs").unwrap().units[2]
        .stars
        .push((InputTarget::Missing, ExportDomain::Macro));
    graph.rebuild(&lookup());
    assert_eq!(
        graph.binding_in(
            "lib.rs",
            SourceModuleId(0),
            BindingId(1),
            ExportDomain::Macro
        ),
        BindingResult::Incomplete
    );
}

#[test]
fn nested_modules_and_domains_roundtrip_retarget_and_disappear_with_their_source() {
    let db = crate::Database::open_in_memory().unwrap();
    db.conn().execute_batch("INSERT INTO files (path,hash,language,last_indexed) VALUES ('lib.rs','current','rust',0);").unwrap();
    let mut graph = ModuleGraph::default();
    graph.inputs.insert("lib.rs".into(), model());
    graph.rebuild(&lookup());
    graph.persist(db.conn()).unwrap();
    let mut cold = ModuleGraph::default();
    cold.load(db.conn()).unwrap();
    cold.rebuild(&lookup());
    assert_eq!(
        cold.binding_in(
            "lib.rs",
            SourceModuleId(0),
            BindingId(0),
            ExportDomain::Macro
        ),
        BindingResult::Bound(73)
    );
    for export in &mut cold.inputs.get_mut("lib.rs").unwrap().units[0].exports {
        export.target = InputTarget::Declaration(76);
    }
    cold.rebuild(&lookup());
    assert_eq!(
        cold.binding("lib.rs", BindingId(0), false),
        BindingResult::Bound(76)
    );
    db.conn()
        .execute("UPDATE files SET hash='changed' WHERE path='lib.rs'", [])
        .unwrap();
    let mut stale = ModuleGraph::default();
    stale.load(db.conn()).unwrap();
    stale.rebuild(&lookup());
    assert_eq!(
        stale.binding("lib.rs", BindingId(0), false),
        BindingResult::Missing
    );
    assert!(stale.units.is_empty());
    cold.inputs.clear();
    cold.rebuild(&lookup());
    assert_eq!(
        cold.binding("lib.rs", BindingId(0), false),
        BindingResult::Missing
    );
    assert!(cold.modules.is_empty());
}

#[test]
fn repeated_import_binding_ids_do_not_silently_choose_the_last_target() {
    let mut input = model();
    input
        .imports
        .push(import(0, ExportDomain::Value, InputTarget::Declaration(78)));
    let mut graph = ModuleGraph::default();
    graph.inputs.insert(input.path.clone(), input);
    graph.rebuild(&lookup());
    assert_eq!(
        graph.binding("lib.rs", BindingId(0), false),
        BindingResult::Ambiguous
    );
    assert_eq!(
        graph.binding("lib.rs", BindingId(0), true),
        BindingResult::Bound(71)
    );
}

#[test]
fn an_uncaptured_explicit_competitor_cannot_attest_a_unique_import_or_export() {
    let mut input = model();
    input
        .imports
        .push(import(0, ExportDomain::Value, InputTarget::Missing));
    input
        .exports
        .push(export("Alias", ExportDomain::Type, InputTarget::Missing));
    input
        .imports
        .push(import(9, ExportDomain::Type, from_unit(0, "Alias")));
    let mut graph = ModuleGraph::default();
    graph.inputs.insert(input.path.clone(), input);
    graph.rebuild(&lookup());
    assert_eq!(
        graph.binding("lib.rs", BindingId(0), false),
        BindingResult::Incomplete
    );
    assert_eq!(
        graph.binding("lib.rs", BindingId(9), true),
        BindingResult::Incomplete
    );
}

#[test]
fn old_boolean_domain_payloads_are_readable_but_cannot_attest_current_bindings() {
    let db = crate::Database::open_in_memory().unwrap();
    db.conn().execute_batch("INSERT INTO files (path,hash,language,last_indexed) VALUES ('old.ts','current','typescript',0);").unwrap();
    let payload = serde_json::json!([{
        "binding_epoch": 2, "path": "old.ts", "content_hash": "current",
        "exports": [{"name": "Item", "type_space": true, "target": {"Declaration": 71}}],
        "imports": [], "stars": [], "signatures": [], "scoped_declarations": [],
        "paths": {"extensions": [], "substitutions": [], "directory_entry": "index"}
    }]);
    db.conn()
        .execute(
            "INSERT INTO _bearwisdom_meta (key,value) VALUES ('module_bindings_v1',?1)",
            [payload.to_string()],
        )
        .unwrap();
    let mut graph = ModuleGraph::default();
    graph.load(db.conn()).unwrap();
    assert!(graph.inputs.is_empty());
}

#[test]
fn inconsistent_or_duplicate_normalized_source_keys_cannot_alias_a_module_root() {
    let mut graph = ModuleGraph::default();
    graph.inputs.insert("lib.rs".into(), model());
    graph.inputs.insert("./lib.rs".into(), model());
    let mut mismatched = model();
    mismatched.path = "another.rs".into();
    graph.inputs.insert("unrelated.rs".into(), mismatched);
    graph.rebuild(&lookup());
    assert!(graph.paths.is_empty());
    assert!(graph.modules.is_empty());
    assert_eq!(
        graph.binding("lib.rs", BindingId(0), false),
        BindingResult::Missing
    );
    graph.inputs.remove("./lib.rs");
    let mut intruder = model();
    for export in &mut intruder.units[0].exports {
        export.target = InputTarget::Declaration(79);
    }
    graph.inputs.insert("z_intruder.rs".into(), intruder);
    graph.rebuild(&lookup());
    assert_eq!(
        graph.binding("lib.rs", BindingId(0), false),
        BindingResult::Bound(72)
    );
}
