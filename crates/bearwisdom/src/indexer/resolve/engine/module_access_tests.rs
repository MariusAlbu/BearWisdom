use super::*;

#[test]
fn declaration_access_and_source_cursor_are_numeric_and_fail_closed_without_context() {
    let mut graph = ModuleGraph::default();
    graph.modules = vec![
        Module::default(),
        Module {
            parent: Some(ModuleId(0)),
            ..Default::default()
        },
        Module {
            parent: Some(ModuleId(1)),
            ..Default::default()
        },
        Module {
            parent: Some(ModuleId(0)),
            ..Default::default()
        },
    ];
    graph
        .visibility
        .insert(Entity::Declaration(71), Scope::Within(ModuleId(1)));
    graph
        .visibility
        .insert(Entity::Declaration(72), Scope::Unknown);
    graph
        .visibility
        .insert(Entity::Declaration(73), Scope::Public);
    let site = Site {
        spans: vec![
            (0, 100, ModuleId(0)),
            (10, 50, ModuleId(1)),
            (20, 30, ModuleId(2)),
            (60, 90, ModuleId(3)),
        ],
        ..Default::default()
    };
    assert!(!graph.declaration_access(71, site.module()));
    for (byte, allowed) in [
        (15, true),
        (25, true),
        (65, false),
        (50, false),
        (99, false),
        (100, false),
        (15, true),
    ] {
        site.set_cursor(byte);
        assert_eq!(graph.declaration_access(71, site.module()), allowed);
        assert!(!graph.declaration_access(72, site.module()));
        assert!(graph.declaration_access(73, site.module()));
    }
}

#[test]
fn member_access_recipes_roundtrip_and_rebuild_without_stale_guards() {
    use super::super::super::testkit::{sym, Lookup};
    let lookup = Lookup::new().with(sym(71, "read", "Doc.read", "method", "lib.rs"));
    let mut graph = ModuleGraph::default();
    for scope in [
        Some(InputTarget::LocalNamespace(SourceModuleId(0))),
        None,
        Some(InputTarget::Missing),
    ] {
        let expected = scope.is_none();
        let input = ModuleInput {
            path: "lib.rs".into(),
            source_spans: vec![(SourceModuleId(0), 0, 10)],
            declaration_access: vec![InputTarget::Access {
                target: Box::new(InputTarget::Declaration(71)),
                scope: scope.map(Box::new),
                origin: SourceModuleId(0),
                declaration: true,
            }],
            ..Default::default()
        };
        let input: ModuleInput =
            serde_json::from_str(&serde_json::to_string(&input).unwrap()).unwrap();
        graph.inputs.insert("lib.rs".into(), input);
        graph.rebuild(&lookup);
        assert_eq!(graph.declaration_access(71, None), expected);
        let mut cold = ModuleGraph {
            inputs: graph.inputs.clone(),
            ..Default::default()
        };
        cold.rebuild(&lookup);
        assert_eq!(cold.declaration_access(71, None), expected);
        let site = cold.site("lib.rs");
        site.set_cursor(5);
        assert!(site.module().is_some());
    }
    graph.inputs.clear();
    graph.rebuild(&lookup);
    assert!(graph.visibility.is_empty());
    assert!(graph.site("lib.rs").spans.is_empty());
}

#[test]
fn access_uses_module_ancestry_and_never_an_unrelated_root() {
    let mut graph = ModuleGraph::default();
    graph.modules = vec![
        Module::default(),
        Module {
            parent: Some(ModuleId(0)),
            ..Default::default()
        },
        Module::default(),
    ];
    graph.targets.guards.push(Guard {
        target: Target::Declaration(71),
        scope: Some(Target::Namespace(ModuleId(0))),
        origin: ModuleId(0),
        declaration: true,
    });
    assert!(graph
        .export_access(Target::Access(0), Some(ModuleId(1)))
        .is_some());
    assert!(graph
        .export_access(Target::Access(0), Some(ModuleId(2)))
        .is_none());
    assert!(graph.export_access(Target::Access(0), None).is_none());
    graph.targets.guards[0].scope = Some(Target::Namespace(ModuleId(2)));
    assert!(
        graph
            .export_access(Target::Access(0), Some(ModuleId(2)))
            .is_none(),
        "restriction must be an ancestor of the declaration"
    );
}

#[test]
fn public_alias_cannot_widen_a_private_terminal_declaration() {
    let mut graph = ModuleGraph::default();
    graph.modules.push(Module::default());
    graph
        .visibility
        .insert(Entity::Declaration(71), Scope::Within(ModuleId(0)));
    assert!(!graph.valid_reexport(BindingResult::Bound(71), Scope::Public));
    assert!(graph.valid_reexport(BindingResult::Bound(71), Scope::Within(ModuleId(0))));
    graph
        .visibility
        .insert(Entity::Declaration(71), Scope::Unknown);
    assert!(!graph.valid_reexport(BindingResult::Bound(71), Scope::Within(ModuleId(0))));
}

#[test]
fn access_rebuilds_drop_stale_visibility_and_retarget_exact_ids() {
    use super::super::super::{
        module_input::{InputBinding, InputExport},
        testkit::{sym, Lookup},
    };
    let lookup = Lookup::new()
        .with(sym(71, "Item", "Item", "struct", "lib.rs"))
        .with(sym(72, "Item", "Item", "struct", "lib.rs"));
    let mut graph = ModuleGraph::default();
    graph.inputs.insert(
        "consumer.rs".into(),
        ModuleInput {
            path: "consumer.rs".into(),
            imports: vec![InputBinding {
                binding: 0,
                domain: ExportDomain::Type,
                target: InputTarget::From {
                    module: "./lib.rs".into(),
                    name: "Item".into(),
                },
            }],
            ..Default::default()
        },
    );
    for (public, id, present) in [
        (false, 71, true),
        (true, 71, true),
        (false, 71, true),
        (true, 72, true),
        (true, 72, false),
    ] {
        let target = InputTarget::Access {
            target: Box::new(InputTarget::Declaration(id)),
            scope: (!public).then(|| Box::new(InputTarget::LocalNamespace(SourceModuleId(0)))),
            origin: SourceModuleId(0),
            declaration: true,
        };
        graph.inputs.insert(
            "lib.rs".into(),
            ModuleInput {
                path: "lib.rs".into(),
                exports: present
                    .then(|| InputExport {
                        name: "Item".into(),
                        domain: ExportDomain::Type,
                        target,
                    })
                    .into_iter()
                    .collect(),
                ..Default::default()
            },
        );
        graph.rebuild(&lookup);
        let expected = if public && present {
            BindingResult::Bound(id)
        } else {
            BindingResult::Missing
        };
        assert_eq!(graph.binding("consumer.rs", BindingId(0), true), expected);
        let mut fresh = ModuleGraph {
            inputs: graph.inputs.clone(),
            ..Default::default()
        };
        fresh.rebuild(&lookup);
        assert_eq!(fresh.binding("consumer.rs", BindingId(0), true), expected);
    }
}

#[test]
fn declaration_only_scope_paths_reject_aliases_duplicates_cycles_and_wrong_parent_ids() {
    let mut graph = ModuleGraph::default();
    graph.modules = vec![
        Module::default(),
        Module {
            parent: Some(ModuleId(0)),
            ..Default::default()
        },
    ];
    let name = ExportNameId(1);
    let binding = Target::Binding(ModuleId(0), BindingId(0), ExportDomain::Type);
    graph.modules[0]
        .exports
        .insert((name, ExportDomain::Type), vec![Target::Access(0)]);
    graph.modules[0].imports.insert(
        (BindingId(0), ExportDomain::Type),
        vec![Target::Namespace(ModuleId(1))],
    );
    graph.targets.guards.push(Guard {
        target: binding,
        scope: None,
        origin: ModuleId(0),
        declaration: true,
    });
    let base = Target::Namespace(ModuleId(0));
    assert_eq!(graph.declared_scope(base, &[name]), Some(ModuleId(1)));
    graph.targets.guards[0].declaration = false;
    assert_eq!(graph.declared_scope(base, &[name]), None);
    graph.targets.guards[0].declaration = true;
    graph.modules[1].parent = None;
    assert_eq!(graph.declared_scope(base, &[name]), None);
    graph.modules[1].parent = Some(ModuleId(0));
    graph.modules[0]
        .imports
        .insert((BindingId(0), ExportDomain::Type), vec![binding]);
    assert_eq!(
        graph.declared_scope(base, &[name]),
        None,
        "cycles must exhaust a finite work budget"
    );
    graph.modules[0].imports.insert(
        (BindingId(0), ExportDomain::Type),
        vec![Target::Namespace(ModuleId(1)); 2],
    );
    assert_eq!(graph.declared_scope(base, &[name]), None);
}
