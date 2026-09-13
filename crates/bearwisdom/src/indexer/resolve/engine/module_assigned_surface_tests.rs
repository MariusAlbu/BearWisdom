use super::super::super::testkit::{sym, Lookup};
use super::*;

/// A value declaration whose declared type owns two members.
fn value_with_members() -> Lookup {
    let lookup = Lookup::new()
        .with(sym(313931, "path", "path", "const", "path.d.ts"))
        .with_member_id(
            313914,
            sym(
                313916,
                "join",
                "path.PlatformPath.join",
                "method",
                "path.d.ts",
            ),
        )
        .with_member_id(
            313914,
            sym(
                313917,
                "dirname",
                "path.PlatformPath.dirname",
                "method",
                "path.d.ts",
            ),
        );
    let declared = lookup
        .type_arena()
        .unwrap()
        .decl("path.PlatformPath", 313914);
    lookup.with_field_type_id_of(313931, declared)
}

#[test]
fn an_assigned_value_publishes_its_declared_types_members_from_one_shared_surface() {
    let lookup = value_with_members();
    let mut graph = ModuleGraph::default();
    graph.modules = (0..2).map(|_| Module::default()).collect();
    for module in &mut graph.modules {
        module
            .assignments
            .insert(ExportDomain::Value, vec![Target::Declaration(313931)]);
    }
    graph.install_assigned_surfaces(&lookup);

    let surface = graph.modules[0].assigned_surface[&ExportDomain::Value];
    assert_eq!(
        graph.modules[1].assigned_surface[&ExportDomain::Value],
        surface,
        "two modules assigning one declaration share a surface"
    );
    for (name, row) in [("join", 313916), ("dirname", 313917)] {
        let name = graph.export_name(name).unwrap();
        assert_eq!(
            graph.select_export(surface, name),
            BindingResult::Bound(row)
        );
    }
    assert!(graph.export_name("nope").is_none());
}

#[test]
fn a_declaration_without_member_bearing_type_allocates_no_surface() {
    let lookup = Lookup::new().with(sym(71, "shim", "shim", "const", "a.d.ts"));
    let memberless = lookup.type_arena().unwrap().decl("Shim", 72);
    let lookup = lookup.with_field_type_id_of(71, memberless);
    let mut graph = ModuleGraph::default();
    graph.modules.push(Module::default());
    graph.modules[0]
        .assignments
        .insert(ExportDomain::Value, vec![Target::Declaration(71)]);
    graph.install_assigned_surfaces(&lookup);
    assert!(graph.modules[0].assigned_surface.is_empty());
    assert_eq!(graph.modules.len(), 1, "no module for an empty member set");

    // A declaration carrying no declared type at all is the same non-event.
    graph.modules[0]
        .assignments
        .insert(ExportDomain::Value, vec![Target::Declaration(99)]);
    graph.install_assigned_surfaces(&lookup);
    assert!(graph.modules[0].assigned_surface.is_empty());
    assert_eq!(graph.modules.len(), 1);
}
