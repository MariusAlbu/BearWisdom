use super::super::super::module_input::{FileLayout, InputSourceFile, InputUnit};
use super::*;

fn configuration(path: &str) -> ModulePackage {
    use crate::ecosystem::manifest::module_config::ModuleTarget;
    ModulePackage {
        root: String::new(),
        name: "pkg".into(),
        fingerprint: path.into(),
        dependencies: Vec::new(),
        targets: vec![
            ModuleTarget {
                name: "api".into(),
                path: path.into(),
                kind: TargetKind::Library,
                conditional: false,
            },
            ModuleTarget {
                name: "probe".into(),
                path: "benches/probe.rs".into(),
                kind: TargetKind::Development,
                conditional: false,
            },
        ],
    }
}

#[test]
fn current_configuration_retargets_stored_import_ids_and_deleted_roots_do_not_resurrect() {
    use super::super::super::{
        module_input::{InputBinding, InputExport, BINDING_EPOCH},
        testkit::{sym, Lookup},
    };
    let lookup = Lookup::new()
        .with(sym(71, "Item", "Item", "struct", "a.rs"))
        .with(sym(72, "Item", "Item", "struct", "b.rs"));
    let db = crate::Database::open_in_memory().unwrap();
    let mut graph = ModuleGraph::default();
    graph.configuration = Some(vec![configuration("a.rs")]);
    for (path, id) in [("a.rs", 71), ("b.rs", 72), ("benches/probe.rs", 0)] {
        db.conn()
            .execute(
                "INSERT INTO files (path,hash,language,last_indexed) VALUES (?1,'same','rust',0)",
                [path],
            )
            .unwrap();
        graph.inputs.insert(
            path.into(),
            ModuleInput {
                path: path.into(),
                content_hash: "same".into(),
                binding_epoch: BINDING_EPOCH,
                exports: vec![InputExport {
                    name: "Item".into(),
                    domain: ExportDomain::Type,
                    target: InputTarget::Declaration(id),
                }],
                ..Default::default()
            },
        );
    }
    graph
        .inputs
        .get_mut("benches/probe.rs")
        .unwrap()
        .imports
        .push(InputBinding {
            binding: 0,
            domain: ExportDomain::Type,
            target: InputTarget::Select {
                base: Box::new(InputTarget::ExternalRoot("api".into())),
                selectors: vec![("Item".into(), ExportDomain::Type)],
            },
        });
    graph.rebuild(&lookup);
    assert_eq!(
        graph.binding("benches/probe.rs", BindingId(0), true),
        BindingResult::Bound(71)
    );
    graph.persist(db.conn()).unwrap();
    let mut restored = ModuleGraph::default();
    restored.load(db.conn()).unwrap();
    restored.rebuild(&lookup);
    assert_eq!(
        restored.binding("benches/probe.rs", BindingId(0), true),
        BindingResult::Bound(71)
    );
    let mut current = ModuleGraph::default();
    current.configuration = Some(vec![configuration("b.rs")]);
    current.load(db.conn()).unwrap();
    current.rebuild(&lookup);
    assert_eq!(
        current.binding("benches/probe.rs", BindingId(0), true),
        BindingResult::Bound(72)
    );
    db.conn()
        .execute("DELETE FROM files WHERE path='b.rs'", [])
        .unwrap();
    let mut deleted = ModuleGraph::default();
    deleted.configuration = Some(vec![configuration("b.rs")]);
    deleted.load(db.conn()).unwrap();
    deleted.rebuild(&lookup);
    assert_eq!(
        deleted.binding("benches/probe.rs", BindingId(0), true),
        BindingResult::Incomplete
    );
    let mut removed = ModuleGraph::default();
    removed.configuration = Some(Vec::new());
    removed.load(db.conn()).unwrap();
    assert!(removed.configuration.as_ref().unwrap().is_empty());
}

#[test]
fn source_paths_follow_root_inline_and_non_mod_file_ancestry() {
    let mut input = ModuleInput {
        path: "custom/entry.rs".into(),
        file_layout: Some(FileLayout {
            extension: ".rs".into(),
            directory_entry: "mod.rs".into(),
        }),
        ..Default::default()
    };
    let mut file = InputSourceFile {
        owner: SourceModuleId(0),
        name: "child".into(),
        path: None,
    };
    assert_eq!(
        candidates(&input, &file, true),
        ["custom/child.rs", "custom/child/mod.rs"]
    );
    assert_eq!(
        candidates(&input, &file, false),
        ["custom/entry/child.rs", "custom/entry/child/mod.rs"]
    );
    input.units.push(InputUnit {
        id: SourceModuleId(1),
        source_name: Some("inline".into()),
        ..Default::default()
    });
    file.owner = SourceModuleId(1);
    file.path = Some("other.rs".into());
    assert_eq!(
        candidates(&input, &file, false),
        ["custom/entry/inline/other.rs"]
    );
    input.units[0].source_path = Some("redirected".into());
    assert_eq!(
        candidates(&input, &file, false),
        ["custom/redirected/other.rs"]
    );
    file.owner = SourceModuleId(0);
    assert_eq!(candidates(&input, &file, false), ["custom/other.rs"]);
}
