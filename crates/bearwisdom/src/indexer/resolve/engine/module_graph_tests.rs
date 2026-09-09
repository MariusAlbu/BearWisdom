use super::*;

#[test]
fn wildcard_name_exclusions_are_language_data_not_a_global_default_rule() {
    use super::super::{
        module_input::{InputBinding, InputExport},
        testkit::{sym, Lookup},
    };
    let lookup = Lookup::new().with(sym(71, "default", "default", "function", "provider.rs"));
    let mut graph = ModuleGraph::default();
    graph.inputs.insert(
        "provider.rs".into(),
        ModuleInput {
            path: "provider.rs".into(),
            exports: vec![InputExport {
                name: "default".into(),
                domain: ExportDomain::Value,
                target: InputTarget::Declaration(71),
            }],
            ..Default::default()
        },
    );
    graph.inputs.insert(
        "barrel.rs".into(),
        ModuleInput {
            path: "barrel.rs".into(),
            stars: vec![("./provider.rs".into(), false)],
            ..Default::default()
        },
    );
    graph.inputs.insert(
        "consumer.rs".into(),
        ModuleInput {
            path: "consumer.rs".into(),
            imports: vec![InputBinding {
                binding: 0,
                domain: ExportDomain::Value,
                target: InputTarget::From {
                    module: "./barrel.rs".into(),
                    name: "default".into(),
                },
            }],
            ..Default::default()
        },
    );
    graph.rebuild(&lookup);
    assert_eq!(
        graph.binding("consumer.rs", BindingId(0), false),
        BindingResult::Bound(71)
    );
}

#[test]
fn persisted_module_inputs_require_current_fingerprints_and_never_override_fresh_inputs() {
    let db = crate::Database::open_in_memory().unwrap();
    db.conn().execute_batch("INSERT INTO files (path,hash,language,last_indexed) VALUES
        ('kept.ts','a','typescript',0),('stale.ts','b','typescript',0),('fresh.ts','a','typescript',0);").unwrap();
    let mut graph = ModuleGraph::default();
    for path in ["kept.ts", "stale.ts", "deleted.ts", "fresh.ts"] {
        graph.inputs.insert(
            path.into(),
            ModuleInput {
                path: path.into(),
                content_hash: "a".into(),
                binding_epoch: super::super::module_input::BINDING_EPOCH,
                ..Default::default()
            },
        );
    }
    graph.persist(db.conn()).unwrap();
    let mut restored = ModuleGraph::default();
    restored.inputs.insert(
        "fresh.ts".into(),
        ModuleInput {
            path: "fresh.ts".into(),
            content_hash: "new-parse".into(),
            ..Default::default()
        },
    );
    restored.load(db.conn()).unwrap();
    assert_eq!(restored.inputs.len(), 2);
    assert!(restored.inputs.contains_key("kept.ts"));
    assert_eq!(restored.inputs["fresh.ts"].content_hash, "new-parse");
}

#[test]
fn matching_source_hash_does_not_validate_an_older_binding_allocator() {
    let db = crate::Database::open_in_memory().unwrap();
    db.conn().execute_batch("INSERT INTO files (path,hash,language,last_indexed) VALUES ('a.ts','same','typescript',0);").unwrap();
    let mut graph = ModuleGraph::default();
    graph.inputs.insert(
        "a.ts".into(),
        ModuleInput {
            path: "a.ts".into(),
            content_hash: "same".into(),
            ..Default::default()
        },
    );
    graph.persist(db.conn()).unwrap();
    let mut cold = ModuleGraph::default();
    cold.load(db.conn()).unwrap();
    assert!(
        cold.inputs.is_empty(),
        "old BindingIds must not attach to newly captured lexical declarations"
    );
}

#[test]
fn wildcard_conflicts_do_not_choose_the_first_id_and_explicit_exports_win() {
    let name = ExportNameId(1);
    let mut graph = ModuleGraph::default();
    graph.modules = (0..4).map(|_| Module::default()).collect();
    graph.modules[0].stars = star_modules(&[Some(1), Some(2)]);
    graph.modules[1]
        .exports
        .insert((name, ExportDomain::Value), vec![Target::Declaration(71)]);
    graph.modules[2]
        .exports
        .insert((name, ExportDomain::Value), vec![Target::Declaration(72)]);
    assert_eq!(
        graph.export(
            ModuleId(0),
            name,
            ExportDomain::Value,
            &mut FxHashSet::default()
        ),
        BindingResult::Ambiguous
    );
    graph.modules[0]
        .exports
        .insert((name, ExportDomain::Value), vec![Target::Declaration(73)]);
    graph.modules[3].stars = star_modules(&[Some(0)]);
    graph.modules[0].stars.extend(star_modules(&[Some(3)]));
    assert_eq!(
        graph.export(
            ModuleId(3),
            name,
            ExportDomain::Value,
            &mut FxHashSet::default()
        ),
        BindingResult::Bound(73)
    );
    graph.modules[0]
        .exports
        .insert((name, ExportDomain::Value), vec![Target::Missing]);
    assert_eq!(
        graph.export(
            ModuleId(3),
            name,
            ExportDomain::Value,
            &mut FxHashSet::default()
        ),
        BindingResult::Missing
    );
}

#[test]
fn cycles_same_declaration_diamonds_default_and_type_space_are_distinct() {
    let name = ExportNameId(1);
    let mut graph = ModuleGraph::default();
    graph.modules = (0..3).map(|_| Module::default()).collect();
    graph.modules[0].stars = star_modules(&[Some(1), Some(2)]);
    graph.modules[1].stars = star_modules(&[Some(0)]);
    graph.modules[1]
        .exports
        .insert((name, ExportDomain::Type), vec![Target::Declaration(71)]);
    graph.modules[2]
        .exports
        .insert((name, ExportDomain::Type), vec![Target::Declaration(71)]);
    graph.modules[2].exports.insert(
        (ExportNameId(0), ExportDomain::Type),
        vec![Target::Declaration(72)],
    );
    graph.modules[0].wildcard_exclusions.insert(ExportNameId(0));
    assert_eq!(
        graph.export(
            ModuleId(0),
            name,
            ExportDomain::Type,
            &mut FxHashSet::default()
        ),
        BindingResult::Bound(71)
    );
    assert_eq!(
        graph.export(
            ModuleId(0),
            name,
            ExportDomain::Value,
            &mut FxHashSet::default()
        ),
        BindingResult::Missing
    );
    assert_eq!(
        graph.export(
            ModuleId(0),
            ExportNameId(0),
            ExportDomain::Type,
            &mut FxHashSet::default()
        ),
        BindingResult::Missing
    );
    graph.modules[0].stars.extend(star_modules(&[None]));
    assert_eq!(
        graph.export(
            ModuleId(0),
            name,
            ExportDomain::Type,
            &mut FxHashSet::default()
        ),
        BindingResult::Incomplete
    );
}

#[test]
fn deep_shared_barrel_diamonds_visit_each_export_key_once() {
    let name = ExportNameId(1);
    let mut graph = ModuleGraph::default();
    graph.modules = (0..1_000).map(|_| Module::default()).collect();
    for index in 0..998 {
        graph.modules[index].stars = star_modules(&[Some(index + 1), Some(index + 2)]);
    }
    graph.modules[998].stars = star_modules(&[Some(999)]);
    graph.modules[999]
        .exports
        .insert((name, ExportDomain::Value), vec![Target::Declaration(71)]);
    let mut visited = FxHashSet::default();
    assert_eq!(
        graph.export(ModuleId(0), name, ExportDomain::Value, &mut visited),
        BindingResult::Bound(71)
    );
    assert_eq!(
        visited.len(),
        1_000,
        "shared paths do not multiply work or exhaust a recursive depth limit"
    );
}

fn star_modules(ids: &[Option<usize>]) -> Vec<(Target, ExportDomain)> {
    ids.iter()
        .flat_map(|id| {
            [ExportDomain::Value, ExportDomain::Type].map(|domain| {
                (
                    id.map(|id| Target::Namespace(ModuleId(id)))
                        .unwrap_or(Target::Missing),
                    domain,
                )
            })
        })
        .collect()
}
