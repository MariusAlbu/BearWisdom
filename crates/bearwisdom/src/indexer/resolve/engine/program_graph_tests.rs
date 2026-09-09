use super::super::{
    module_graph::ModuleGraph,
    program_input::{Input, Part},
    testkit::{sym, Lookup},
};
use super::*;
use crate::indexer::programs::ProgramSource;
use crate::types::SymbolKind;

#[test]
fn source_binding_order_is_attested_complete_and_snapshot_owned() {
    let (mut graph, lookup) = fixture();
    assert!(graph
        .programs
        .source_binding_order(graph.programs.program("a").unwrap())
        .is_none());
    for order in [
        vec!["a.ts", "shared.ts"],
        vec!["a.ts"],
        vec!["a.ts", "a.ts"],
        vec!["missing.ts", "shared.ts"],
    ] {
        graph.programs.configuration.as_mut().unwrap()[0].source_binding_order =
            Some(order.iter().map(|s| (*s).into()).collect());
        graph.rebuild(&lookup);
        let program = graph.programs.program("a").unwrap();
        let bound = graph.programs.source_binding_order(program);
        if order == ["a.ts", "shared.ts"] {
            let bound = bound.unwrap();
            assert_eq!(bound[&graph.programs.source(program, "a.ts").unwrap()], 0);
            assert_eq!(
                bound[&graph.programs.source(program, "shared.ts").unwrap()],
                1
            );
            graph.rebuild(&lookup);
            assert!(graph.programs.source_binding_order(program).is_none());
        } else {
            assert!(bound.is_none());
        }
    }
}

#[test]
fn program_rebuilds_invalidate_nominal_contexts_even_for_the_same_physical_rows() {
    let (mut graph, lookup) = fixture();
    let arena = crate::type_checker::core::types::TypeArena::new();
    let a = graph.programs.program("a").unwrap();
    let b = graph.programs.program("b").unwrap();
    let context_a = graph.programs.nominal_context(a).unwrap();
    let context_b = graph.programs.nominal_context(b).unwrap();
    let left = arena.decl_in(context_a, "Catalog", 71);
    let right = arena.decl_in(context_b, "Catalog", 71);
    assert_ne!(left, right);
    graph.rebuild(&lookup);
    assert!(graph.programs.nominal_context(a).is_none());
    let a = graph.programs.program("a").unwrap();
    let context = graph.programs.nominal_context(a).unwrap();
    assert_ne!(context_a, context);
    assert!(!arena.accepts_nominal_context(left, Some(context)));
    graph.inputs.remove("a.ts");
    graph.rebuild(&lookup);
    assert!(graph
        .programs
        .nominal_context(graph.programs.program("a").unwrap())
        .is_none());
    assert!(graph
        .programs
        .nominal_context(graph.programs.program("b").unwrap())
        .is_some());
}

fn part(row: i64, kind: SymbolKind, type_space: bool) -> Part {
    Part {
        unit: super::super::module_input::SourceModuleId(0),
        name: "Catalog".into(),
        binding: Some(row as usize),
        declaration: Some(row),
        kind,
        type_space,
        parameters: if type_space { vec!["T".into()] } else { vec![] },
        plain_parameters: true,
        members: vec![],
        plain_merge: true,
        plain_header: true,
        type_heritage: false,
        surface: None,
    }
}
fn input(path: &str, parts: Vec<Part>) -> ModuleInput {
    ModuleInput {
        path: path.into(),
        content_hash: "source".into(),
        binding_epoch: super::super::module_input::BINDING_EPOCH,
        globals: Some(Input {
            complete: true,
            roots: parts,
            merge_rules: vec![
                (SymbolKind::Interface, SymbolKind::Interface),
                (SymbolKind::Class, SymbolKind::Interface),
            ],
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn config(key: &str, paths: &[&str]) -> Program {
    Program {
        key: key.into(),
        fingerprint: key.into(),
        complete: true,
        callable_policy: None,
        compiler_intrinsics: None,
        source_binding_order: None,
        sources: paths
            .iter()
            .map(|path| ProgramSource {
                path: (*path).into(),
                content_hash: "source".into(),
                scope: SourceScope::Syntax,
            })
            .collect(),
    }
}
fn fixture() -> (ModuleGraph, Lookup) {
    let mut graph = ModuleGraph::default();
    let mut lookup = Lookup::new();
    for (path, row) in [("shared.ts", 71), ("a.ts", 72), ("b.ts", 73)] {
        graph.inputs.insert(
            path.into(),
            input(path, vec![part(row, SymbolKind::Interface, true)]),
        );
        lookup = lookup.with(sym(row, "Catalog", "Catalog", "interface", path));
    }
    graph.programs.configuration = Some(vec![
        config("a", &["shared.ts", "a.ts"]),
        config("b", &["shared.ts", "b.ts"]),
    ]);
    graph.rebuild(&lookup);
    (graph, lookup)
}
fn result(graph: &Graph, key: &str) -> Result {
    let id = graph.program(key).unwrap();
    graph.global(id, graph.name(id, "Catalog").unwrap(), true)
}
fn rows(graph: &Graph, key: &str) -> Vec<i64> {
    let Result::Bound(id) = result(graph, key) else {
        panic!("{key}: {:?}", result(graph, key));
    };
    graph
        .group(id)
        .unwrap()
        .parts
        .iter()
        .map(|p| p.declaration)
        .collect()
}

#[test]
fn overlapping_programs_have_distinct_source_name_and_group_identities() {
    let (graph, _) = fixture();
    let g = &graph.programs;
    assert_eq!(rows(g, "a"), [71, 72]);
    assert_eq!(rows(g, "b"), [71, 73]);
    let a = g.program("a").unwrap();
    let b = g.program("b").unwrap();
    let sa = g.source(a, "shared.ts").unwrap();
    let sb = g.source(b, "shared.ts").unwrap();
    assert_ne!(sa, sb);
    assert_ne!(result(g, "a"), result(g, "b"));
    assert_eq!(g.binding(sa, BindingId(71), true), result(g, "a"));
    assert_eq!(g.binding(sb, BindingId(71), true), result(g, "b"));
    assert_eq!(
        g.global(b, g.name(a, "Catalog").unwrap(), true),
        Result::Unconfigured
    );
    let Result::Bound(group) = result(g, "a") else {
        unreachable!()
    };
    let parts = &g.group(group).unwrap().parts;
    assert_eq!(parts[0].source, sa);
    assert_eq!(parts[0].parameters, parts[1].parameters);
}

#[test]
fn old_or_other_snapshot_handles_cannot_alias_new_numeric_slots() {
    let (mut graph, lookup) = fixture();
    let program = graph.programs.program("a").unwrap();
    let name = graph.programs.name(program, "Catalog").unwrap();
    let source = graph.programs.source(program, "shared.ts").unwrap();
    let Result::Bound(group) = result(&graph.programs, "a") else {
        unreachable!()
    };
    graph.rebuild(&lookup);
    assert_eq!(
        graph.programs.global(program, name, true),
        Result::Unconfigured
    );
    assert_eq!(
        graph.programs.binding(source, BindingId(71), true),
        Result::Unconfigured
    );
    assert!(graph.programs.group(group).is_none());
    let (other, _) = fixture();
    assert!(other.programs.group(group).is_none());
}

#[test]
fn isolated_modules_do_not_contribute_namesakes_but_explicit_augmentations_do() {
    let (mut graph, lookup) = fixture();
    let globals = graph
        .inputs
        .get_mut("a.ts")
        .unwrap()
        .globals
        .as_mut()
        .unwrap();
    globals.isolated = true;
    graph.rebuild(&lookup);
    assert_eq!(rows(&graph.programs, "a"), [71]);
    let globals = graph
        .inputs
        .get_mut("a.ts")
        .unwrap()
        .globals
        .as_mut()
        .unwrap();
    globals.augmentations = std::mem::take(&mut globals.roots);
    for part in &mut globals.augmentations {
        part.unit = super::super::module_input::SourceModuleId(1);
    }
    graph.inputs.get_mut("a.ts").unwrap().units.push(serde_json::from_value(serde_json::json!({
        "id": 1, "parent": 0, "exports": [], "imports": [], "stars": [], "wildcard_exclusions": [],
        "source_scope": { "kind": "Augmentation", "lexical_scope": 1, "range": { "start": 0, "end": 40 },
            "body": { "start": 12, "end": 40 }, "ambient": true, "container_valid": true, "complete": true }
    })).unwrap());
    graph.rebuild(&lookup);
    assert_eq!(rows(&graph.programs, "a"), [71, 72]);
}

#[test]
fn type_and_value_domains_do_not_share_a_logical_binding() {
    let (mut graph, lookup) = fixture();
    graph
        .inputs
        .get_mut("a.ts")
        .unwrap()
        .globals
        .as_mut()
        .unwrap()
        .roots
        .push(part(74, SymbolKind::Variable, false));
    let lookup = lookup.with(sym(74, "Catalog", "Catalog", "variable", "a.ts"));
    graph.rebuild(&lookup);
    let g = &graph.programs;
    let a = g.program("a").unwrap();
    let name = g.name(a, "Catalog").unwrap();
    assert_eq!(rows(g, "a"), [71, 72]);
    let Result::Bound(value) = g.global(a, name, false) else {
        panic!("value missing");
    };
    assert_eq!(
        g.group(value)
            .unwrap()
            .parts
            .iter()
            .map(|p| p.declaration)
            .collect::<Vec<_>>(),
        [74]
    );
    assert_ne!(g.global(a, name, false), result(g, "a"));
}

#[test]
fn missing_stale_deleted_or_unknown_provider_is_not_absence_of_competitors() {
    for fault in 0..6 {
        let (mut graph, lookup) = fixture();
        match fault {
            0 => {
                graph.inputs.remove("a.ts");
            }
            1 => graph.inputs.get_mut("a.ts").unwrap().content_hash = "edit".into(),
            2 => graph.inputs.get_mut("a.ts").unwrap().binding_epoch = 0,
            3 => graph.inputs.get_mut("a.ts").unwrap().globals = None,
            4 => {
                graph.programs.configuration.as_mut().unwrap()[0].sources[1].scope =
                    SourceScope::Unknown
            }
            _ => graph.programs.configuration.as_mut().unwrap()[0].complete = false,
        }
        graph.rebuild(&lookup);
        assert_eq!(
            result(&graph.programs, "a"),
            Result::Incomplete,
            "fault {fault}"
        );
        assert_eq!(rows(&graph.programs, "b"), [71, 73]);
    }
}

#[test]
fn incompatible_or_unproven_merge_evidence_never_pairs_generic_ids() {
    for fault in 0..5 {
        let (mut graph, lookup) = fixture();
        let part = &mut graph
            .inputs
            .get_mut("a.ts")
            .unwrap()
            .globals
            .as_mut()
            .unwrap()
            .roots[0];
        match fault {
            0 => part.parameters.push("U".into()),
            1 => part.parameters[0] = "U".into(),
            2 => part.kind = SymbolKind::TypeAlias,
            3 => part.plain_parameters = false,
            _ => part.declaration = Some(73), // live row, wrong physical source
        }
        graph.rebuild(&lookup);
        assert_eq!(
            result(&graph.programs, "a"),
            if fault < 3 {
                Result::Ambiguous
            } else {
                Result::Incomplete
            }
        );
    }
}

#[test]
fn configured_module_detection_controls_root_visibility_and_augmentation_legality() {
    let (mut graph, lookup) = fixture();
    graph.programs.configuration.as_mut().unwrap()[0].sources[1].scope = SourceScope::Module;
    graph.rebuild(&lookup);
    assert_eq!(rows(&graph.programs, "a"), [71]);
    let globals = graph
        .inputs
        .get_mut("a.ts")
        .unwrap()
        .globals
        .as_mut()
        .unwrap();
    globals.augmentations = std::mem::take(&mut globals.roots);
    for part in &mut globals.augmentations {
        part.unit = super::super::module_input::SourceModuleId(1);
    }
    graph.inputs.get_mut("a.ts").unwrap().units.push(serde_json::from_value(serde_json::json!({
        "id": 1, "parent": 0, "exports": [], "imports": [], "stars": [], "wildcard_exclusions": [],
        "source_scope": { "kind": "Augmentation", "lexical_scope": 1, "range": { "start": 0, "end": 40 },
            "body": { "start": 12, "end": 40 }, "ambient": true, "container_valid": true, "complete": true }
    })).unwrap());
    graph.rebuild(&lookup);
    assert_eq!(rows(&graph.programs, "a"), [71, 72]);
    graph.programs.configuration.as_mut().unwrap()[0].sources[1].scope = SourceScope::Syntax;
    graph.rebuild(&lookup);
    assert_eq!(result(&graph.programs, "a"), Result::Incomplete);
}

#[test]
fn duplicate_config_keys_and_memberships_never_choose_a_winner() {
    let (mut graph, lookup) = fixture();
    let duplicate = graph.programs.configuration.as_ref().unwrap()[0].clone();
    graph
        .programs
        .configuration
        .as_mut()
        .unwrap()
        .push(duplicate);
    graph.rebuild(&lookup);
    assert!(graph.programs.program("a").is_none());
    graph.programs.configuration.as_mut().unwrap().pop();
    let duplicate = graph.programs.configuration.as_ref().unwrap()[0].sources[0].clone();
    graph.programs.configuration.as_mut().unwrap()[0]
        .sources
        .push(duplicate);
    graph.rebuild(&lookup);
    assert_eq!(result(&graph.programs, "a"), Result::Incomplete);
}

#[test]
fn unverified_member_collisions_and_inheritance_do_not_become_legal_merge_proofs() {
    for inherited in [false, true] {
        let (mut graph, lookup) = fixture();
        graph
            .inputs
            .get_mut("shared.ts")
            .unwrap()
            .globals
            .as_mut()
            .unwrap()
            .roots[0]
            .members = vec!["value".into()];
        let part = &mut graph
            .inputs
            .get_mut("a.ts")
            .unwrap()
            .globals
            .as_mut()
            .unwrap()
            .roots[0];
        if inherited {
            part.plain_merge = false;
        } else {
            part.members = vec!["value".into()];
        }
        graph.rebuild(&lookup);
        assert_eq!(result(&graph.programs, "a"), Result::Incomplete);
        assert_eq!(rows(&graph.programs, "b"), [71, 73]);
    }
}

#[test]
fn configuration_and_source_evidence_roundtrip_and_fresh_configuration_wins() {
    let (graph, lookup) = fixture();
    let db = crate::Database::open_in_memory().unwrap();
    for path in ["shared.ts", "a.ts", "b.ts"] {
        db.conn().execute("INSERT INTO files (path,hash,language,last_indexed) VALUES (?1,'source','typescript',0)", [path]).unwrap();
    }
    graph.persist(db.conn()).unwrap();
    let mut cold = ModuleGraph::default();
    cold.load(db.conn()).unwrap();
    cold.rebuild(&lookup);
    assert_eq!(rows(&cold.programs, "a"), [71, 72]);
    assert_eq!(rows(&cold.programs, "b"), [71, 73]);
    let mut changed = ModuleGraph::default();
    changed.programs.configuration = Some(vec![config("a", &["shared.ts", "b.ts"])]);
    changed.load(db.conn()).unwrap();
    changed.rebuild(&lookup);
    assert_eq!(rows(&changed.programs, "a"), [71, 73]);
    db.conn()
        .execute("DELETE FROM files WHERE path='b.ts'", [])
        .unwrap();
    let mut deleted = ModuleGraph::default();
    deleted.load(db.conn()).unwrap();
    deleted.rebuild(&lookup);
    assert_eq!(result(&deleted.programs, "b"), Result::Incomplete);
    assert_eq!(rows(&deleted.programs, "a"), [71, 72]);
    let mut cleared = ModuleGraph::default();
    cleared.programs.configuration = Some(vec![]);
    cleared.load(db.conn()).unwrap();
    cleared.rebuild(&lookup);
    assert!(cleared.programs.program("a").is_none());
}
