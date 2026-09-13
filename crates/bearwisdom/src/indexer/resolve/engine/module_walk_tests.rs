use super::*;

#[test]
fn assigned_entities_keep_default_exports_separate_and_stop_cycles_or_unknown_competitors() {
    let mut graph = ModuleGraph::default();
    graph.modules = (0..3).map(|_| Module::default()).collect();
    let name = ExportNameId(0);
    graph.modules[0]
        .exports
        .insert((name, ExportDomain::Value), vec![Target::Declaration(71)]);
    assert_eq!(
        graph.resolve_target(Target::Assigned(ModuleId(0)), ExportDomain::Value, 0),
        BindingResult::Namespace(ModuleId(0))
    );
    graph.modules[0]
        .assignments
        .insert(ExportDomain::Value, vec![Target::Declaration(72)]);
    assert_eq!(
        graph.resolve_target(Target::Assigned(ModuleId(0)), ExportDomain::Value, 0),
        BindingResult::Bound(72)
    );
    assert_eq!(
        graph.resolve_target(Target::Export(ModuleId(0), name), ExportDomain::Value, 0),
        BindingResult::Bound(71)
    );
    graph.modules[0].assignments.clear();
    graph.modules[0].assignment_parts = vec![ModuleId(1), ModuleId(2)];
    graph.modules[1]
        .assignments
        .insert(ExportDomain::Value, vec![Target::Missing]);
    graph.modules[2]
        .assignments
        .insert(ExportDomain::Value, vec![Target::Declaration(72)]);
    assert!(matches!(
        graph.resolve_target(Target::Assigned(ModuleId(0)), ExportDomain::Value, 0),
        BindingResult::Incomplete | BindingResult::Ambiguous
    ));
    graph.modules[0].assignment_parts.clear();
    graph.modules[0]
        .assignments
        .insert(ExportDomain::Value, vec![Target::Assigned(ModuleId(1))]);
    graph.modules[1]
        .assignments
        .insert(ExportDomain::Value, vec![Target::Assigned(ModuleId(0))]);
    assert_eq!(
        graph.resolve_target(Target::Assigned(ModuleId(0)), ExportDomain::Value, 0),
        BindingResult::Incomplete
    );
}

#[test]
fn an_assigned_values_member_surface_answers_names_the_assignment_alone_cannot() {
    let mut graph = ModuleGraph::default();
    graph.modules = (0..3).map(|_| Module::default()).collect();
    let member = ExportNameId(1);
    let exported = ExportNameId(2);
    // 0 assigns a value; 1 is the namespace it carries; 2 is the value's
    // declared-type member surface.
    graph.modules[1].exports.insert(
        (exported, ExportDomain::Value),
        vec![Target::Declaration(71)],
    );
    graph.modules[2]
        .exports
        .insert((member, ExportDomain::Value), vec![Target::Declaration(72)]);
    graph.modules[0]
        .assignments
        .insert(ExportDomain::Value, vec![Target::Declaration(73)]);
    let select = |graph: &ModuleGraph, name| {
        graph.resolve_target(Target::Export(ModuleId(0), name), ExportDomain::Value, 0)
    };
    assert_eq!(select(&graph, member), BindingResult::Missing);

    graph.modules[0]
        .assigned_surface
        .insert(ExportDomain::Value, ModuleId(2));
    assert_eq!(select(&graph, member), BindingResult::Bound(72));

    graph
        .targets
        .entities
        .push((Target::Declaration(73), Target::Namespace(ModuleId(1))));
    graph.modules[0]
        .assignments
        .insert(ExportDomain::Value, vec![Target::Entity(0)]);
    assert_eq!(select(&graph, exported), BindingResult::Bound(71));
    assert_eq!(select(&graph, member), BindingResult::Bound(72));
    assert_eq!(select(&graph, ExportNameId(3)), BindingResult::Missing);
}

#[test]
fn entity_facets_preserve_callable_and_namespace_targets_without_discarding_barriers() {
    let mut graph = ModuleGraph::default();
    graph.modules.push(Module::default());
    let name = ExportNameId(0);
    graph.modules[0]
        .exports
        .insert((name, ExportDomain::Value), vec![Target::Declaration(72)]);
    graph
        .targets
        .entities
        .push((Target::Declaration(71), Target::Namespace(ModuleId(0))));
    let entity = graph.resolve_target(Target::Entity(0), ExportDomain::Value, 0);
    assert_eq!(entity.declaration(), Some(71));
    assert_eq!(entity.namespace(), Some(ModuleId(0)));
    let path = intern_path(
        &mut graph.targets,
        Target::Entity(0),
        vec![(name, None)],
        None,
    );
    assert_eq!(
        graph.resolve_target(path, ExportDomain::Value, 0),
        BindingResult::Bound(72)
    );
    graph.targets.groups.push(vec![71, 73]);
    graph.targets.entities[0].0 = Target::Overloads(0);
    assert_eq!(
        graph.resolve_target(Target::Entity(0), ExportDomain::Value, 0),
        BindingResult::Entity {
            declaration: None,
            overloads: Some(0),
            namespace: ModuleId(0)
        }
    );
    assert_eq!(
        graph.resolve_target(path, ExportDomain::Value, 0),
        BindingResult::Bound(72)
    );
    graph.targets.entities[0].0 = Target::Incomplete;
    assert_eq!(
        graph.resolve_target(path, ExportDomain::Value, 0),
        BindingResult::Incomplete
    );
    graph.targets.entities[0].0 = Target::Missing;
    assert_eq!(
        graph.resolve_target(Target::Entity(0), ExportDomain::Value, 0),
        BindingResult::Namespace(ModuleId(0))
    );
}

#[test]
fn conflicting_namespace_ids_cannot_collapse_to_a_shared_exported_declaration() {
    let mut graph = ModuleGraph::default();
    graph.modules = (0..3).map(|_| Module::default()).collect();
    let namespace = ExportNameId(1);
    let member = ExportNameId(2);
    graph.modules[0].exports.insert(
        (namespace, ExportDomain::Value),
        vec![
            Target::Namespace(ModuleId(1)),
            Target::Namespace(ModuleId(2)),
        ],
    );
    for module in &mut graph.modules[1..] {
        module
            .exports
            .insert((member, ExportDomain::Value), vec![Target::Declaration(71)]);
    }
    graph.targets.paths.push((
        Target::Export(ModuleId(0), namespace),
        vec![(member, None)],
        None,
    ));
    assert_eq!(
        graph.resolve_target(Target::Path(0), ExportDomain::Value, 0),
        BindingResult::Ambiguous
    );
    graph.modules[0].exports.insert(
        (namespace, ExportDomain::Value),
        vec![Target::Namespace(ModuleId(1))],
    );
    assert_eq!(
        graph.resolve_target(Target::Path(0), ExportDomain::Value, 0),
        BindingResult::Bound(71)
    );
    assert_eq!(
        graph.resolve_target(Target::Path(0), ExportDomain::Type, 0),
        BindingResult::Missing
    );
}

#[test]
fn recursive_qualified_aliases_terminate_without_a_guessed_target() {
    let mut graph = ModuleGraph::default();
    graph.modules.push(Module::default());
    let name = ExportNameId(1);
    graph.modules[0]
        .exports
        .insert((name, ExportDomain::Value), vec![Target::Path(0)]);
    graph
        .targets
        .paths
        .push((Target::Export(ModuleId(0), name), vec![(name, None)], None));
    assert_eq!(
        graph.resolve_target(Target::Path(0), ExportDomain::Value, 0),
        BindingResult::Incomplete
    );
}

#[test]
fn qualified_paths_share_a_total_work_budget_across_recursive_subqueries() {
    let mut graph = ModuleGraph::default();
    graph.modules.push(Module::default());
    graph.targets.paths.push((
        Target::Namespace(ModuleId(0)),
        vec![(ExportNameId(1), None)],
        None,
    ));
    graph.modules[0].exports.insert(
        (ExportNameId(1), ExportDomain::Value),
        vec![Target::Declaration(71)],
    );
    let mut state = WalkState {
        remaining: 3,
        active: FxHashSet::default(),
    };
    assert_eq!(
        graph.resolve_with_state(Target::Path(0), ExportDomain::Value, 0, &mut state),
        BindingResult::Incomplete
    );
    assert_eq!(state.remaining, 0);
    assert!(state.active.is_empty());
    assert_eq!(
        graph.resolve_target(Target::Path(0), ExportDomain::Value, 0),
        BindingResult::Bound(71)
    );
}

#[test]
fn binding_edges_keep_their_domain_and_recursive_bindings_are_bounded() {
    let mut graph = ModuleGraph::default();
    graph.modules.push(Module::default());
    let target = Target::Binding(ModuleId(0), BindingId(0), ExportDomain::Type);
    graph.modules[0].imports.insert(
        (BindingId(0), ExportDomain::Type),
        vec![Target::Declaration(71)],
    );
    graph.modules[0].imports.insert(
        (BindingId(0), ExportDomain::Value),
        vec![Target::Declaration(72)],
    );
    assert_eq!(
        graph.resolve_target(target, ExportDomain::Value, 0),
        BindingResult::Bound(71)
    );
    graph.modules[0]
        .imports
        .insert((BindingId(0), ExportDomain::Type), vec![target]);
    assert_eq!(
        graph.resolve_target(target, ExportDomain::Value, 0),
        BindingResult::Incomplete
    );
}

#[test]
fn access_context_does_not_escape_through_public_barrels_or_hide_competitors() {
    let mut graph = ModuleGraph::default();
    graph.modules = (0..2).map(|_| Module::default()).collect();
    let name = ExportNameId(1);
    graph.targets.guards.push(access::Guard {
        target: Target::Declaration(71),
        scope: Some(Target::Namespace(ModuleId(0))),
        origin: ModuleId(0),
        declaration: true,
    });
    graph.modules[0]
        .exports
        .insert((name, ExportDomain::Type), vec![Target::Access(0)]);
    graph.modules[1]
        .stars
        .push((Target::Namespace(ModuleId(0)), ExportDomain::Type));
    for (id, base, requester, expected) in [
        (0, 0, 0, BindingResult::Bound(71)),
        (1, 1, 0, BindingResult::Missing),
        (2, 0, 1, BindingResult::Missing),
    ] {
        graph.targets.paths.push((
            Target::Namespace(ModuleId(base)),
            vec![(name, None)],
            Some(ModuleId(requester)),
        ));
        assert_eq!(
            graph.resolve_target(Target::Path(id), ExportDomain::Type, 0),
            expected
        );
    }
    graph.modules[0]
        .exports
        .get_mut(&(name, ExportDomain::Type))
        .unwrap()
        .push(Target::Declaration(72));
    assert_eq!(
        graph.resolve_target(Target::Path(2), ExportDomain::Type, 0),
        BindingResult::Incomplete
    );
}

#[test]
fn a_default_import_of_an_export_assigned_module_is_the_assigned_value() {
    let mut graph = ModuleGraph::default();
    graph.modules = vec![Module::default()];
    let default = ExportNameId(0);
    graph.modules[0]
        .assignments
        .insert(ExportDomain::Value, vec![Target::Declaration(71)]);
    graph.modules[0].wildcard_exclusions.insert(default);
    let select = |graph: &ModuleGraph| {
        graph.resolve_target(Target::Export(ModuleId(0), default), ExportDomain::Value, 0)
    };
    assert_eq!(
        select(&graph),
        BindingResult::Missing,
        "a name the module never spelled as its default stays excluded"
    );

    graph.modules[0].default_name = Some(default);
    assert_eq!(select(&graph), BindingResult::Bound(71));

    // An explicit default export still wins over the assignment.
    graph.modules[0].exports.insert(
        (default, ExportDomain::Value),
        vec![Target::Declaration(72)],
    );
    assert_eq!(select(&graph), BindingResult::Bound(72));
}

#[test]
fn a_module_read_only_in_part_answers_the_exports_it_declared_and_no_others() {
    let mut graph = ModuleGraph::default();
    graph.modules = (0..2).map(|_| Module::default()).collect();
    let declared = ExportNameId(1);
    let absent = ExportNameId(2);
    graph.modules[0]
        .exports
        .insert((declared, ExportDomain::Type), vec![Target::Declaration(71)]);
    graph.modules[0]
        .stars
        .push((Target::Namespace(ModuleId(1)), ExportDomain::Type));
    graph.modules[1]
        .exports
        .insert((absent, ExportDomain::Type), vec![Target::Declaration(72)]);
    let select = |graph: &ModuleGraph, name| {
        graph.resolve_target(Target::Export(ModuleId(0), name), ExportDomain::Type, 0)
    };
    assert_eq!(select(&graph, declared), BindingResult::Bound(71));
    assert_eq!(select(&graph, absent), BindingResult::Bound(72));

    graph.modules[0].incomplete = true;
    assert_eq!(
        select(&graph, declared),
        BindingResult::Bound(71),
        "an entry the binder read is evidence, whatever else it could not read"
    );
    assert_eq!(
        select(&graph, absent),
        BindingResult::Incomplete,
        "a name with no entry cannot fall through an unread surface"
    );

    // A cleanly read module whose wildcard reaches an unread one still cannot
    // conclude absence through it.
    graph.modules[0].incomplete = false;
    graph.modules[1].incomplete = true;
    assert_eq!(select(&graph, declared), BindingResult::Bound(71));
    assert_eq!(select(&graph, absent), BindingResult::Bound(72));
    graph.modules[1]
        .exports
        .remove(&(absent, ExportDomain::Type));
    assert_eq!(select(&graph, absent), BindingResult::Incomplete);
}
