use super::*;

#[test]
fn type_only_import_never_acquires_a_value_target() {
    let import = Import {
        source: crate::indexer::lexical::modules::ImportSource::Named {
            module: "./model".into(),
            name: "Model".into(),
        },
        type_only: true,
        selectors: Vec::new(),
    };
    assert!(matches!(
        imported(
            &import,
            &Default::default(),
            ExportDomain::Value,
            "main.ts",
            &Default::default()
        ),
        InputTarget::Missing
    ));
    for domain in [ExportDomain::Type, ExportDomain::ValueQuery] {
        assert!(matches!(
            imported(
                &import,
                &Default::default(),
                domain,
                "main.ts",
                &Default::default()
            ),
            InputTarget::From { .. }
        ));
    }
    let restored: InputTarget =
        serde_json::from_str(&serde_json::to_string(&InputTarget::Declaration(71)).unwrap())
            .unwrap();
    assert!(matches!(restored, InputTarget::Declaration(71)));
}

fn parsed(source: &str) -> ParsedFile {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("provider.d.ts");
    std::fs::write(&path, source).unwrap();
    crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "provider.d.ts".into(),
            absolute_path: path,
            language: "typescript",
        },
        crate::languages::default_registry(),
        &crate::type_checker::core::types::TypeArena::new(),
    )
    .unwrap()
}

#[test]
fn ambient_module_inputs_keep_scoped_imports_and_exports_in_distinct_units() {
    let file = parsed("declare module 'left' { import { Doc as Item } from 'one'; export { Item as Public }; } declare module 'right' { import { Doc as Item } from 'two'; export { Item as Public }; }");
    let input = capture(&file, &SymbolIds::default()).unwrap();
    assert_eq!(input.units.len(), 2);
    assert!(input.exports.is_empty());
    for (unit, name) in input.units.iter().zip(["left", "right"]) {
        assert_eq!(unit.source_name.as_deref(), Some(name));
        assert_eq!(
            unit.imports.len(),
            3,
            "one binding in separate runtime value/type/value-query domains"
        );
        assert_eq!(unit.exports.len(), 3);
        assert!(unit.exports.iter().all(|e| e.name == "Public"));
        for import in &unit.imports {
            assert!(
                input
                    .imports
                    .iter()
                    .any(|entry| entry.binding == import.binding
                        && entry.domain == import.domain
                        && matches!(entry.target, InputTarget::Binding { module, binding, domain }
                    if module == unit.id && binding == import.binding && domain == import.domain)),
                "file BindingId API must forward to its actual unit"
            );
        }
    }
    assert_ne!(
        input.units[0].imports[0].binding,
        input.units[1].imports[0].binding
    );
    let payload = serde_json::to_value(&input).unwrap();
    let cold: ModuleInput = serde_json::from_value(payload.clone()).unwrap();
    assert_eq!(serde_json::to_value(cold).unwrap(), payload);
}

#[test]
fn parsed_namespace_exports_select_exact_rows_through_cold_reload_edits_and_deletion() {
    use super::super::{
        module_graph::{BindingResult, ModuleGraph},
        testkit::{sym, Lookup},
    };
    let source = "export namespace API { export function make(): void {} } export namespace Other { export function make(): void {} }";
    let provider = parsed(source);
    let consumer_source = "import { API, Other } from './provider'; API.make(); Other.make();";
    let mut consumer = parsed(consumer_source);
    consumer.path = "consumer.ts".into();
    let mut ids = SymbolIds::default();
    ids.set_rows(
        provider.path.clone(),
        (0..provider.symbols.len())
            .map(|slot| slot as i64 + 100)
            .collect(),
    );
    let lookup =
        provider
            .symbols
            .iter()
            .enumerate()
            .fold(Lookup::new(), |lookup, (slot, symbol)| {
                lookup.with(sym(
                    slot as i64 + 100,
                    "poisoned",
                    "poisoned",
                    symbol.kind.as_str(),
                    &provider.path,
                ))
            });
    let rows: Vec<_> = provider
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.name == "make")
        .map(|(slot, _)| slot as i64 + 100)
        .collect();
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0], rows[1]);
    let bindings: Vec<_> = consumer_source
        .match_indices("make()")
        .map(|(byte, _)| consumer.flow.lexical.as_ref().unwrap().module.members[&(byte as u32)])
        .collect();
    let mut graph = ModuleGraph::default();
    graph.inputs.insert(
        consumer.path.clone(),
        capture(&consumer, &SymbolIds::default()).unwrap(),
    );
    graph.rebuild(&lookup);
    assert_eq!(
        graph.binding(&consumer.path, bindings[0], false),
        BindingResult::Missing
    );
    graph
        .inputs
        .insert(provider.path.clone(), capture(&provider, &ids).unwrap());
    graph.rebuild(&lookup);
    for (&binding, &row) in bindings.iter().zip(&rows) {
        assert_eq!(
            graph.binding(&consumer.path, binding, false),
            BindingResult::Bound(row)
        );
    }
    let db = crate::Database::open_in_memory().unwrap();
    for file in [&provider, &consumer] {
        db.conn()
            .execute(
                "INSERT INTO files (path,hash,language,last_indexed) VALUES (?1,?2,'typescript',0)",
                [&file.path, &file.content_hash],
            )
            .unwrap();
    }
    graph.persist(db.conn()).unwrap();
    let mut cold = ModuleGraph::default();
    cold.load(db.conn()).unwrap();
    cold.rebuild(&lookup);
    for (&binding, &row) in bindings.iter().zip(&rows) {
        assert_eq!(
            cold.binding(&consumer.path, binding, false),
            BindingResult::Bound(row)
        );
    }
    let changed = parsed("export namespace API { export function other(): void {} } export namespace Other { export function make(): void {} }");
    let mut changed_ids = SymbolIds::default();
    changed_ids.set_rows(
        changed.path.clone(),
        (0..changed.symbols.len())
            .map(|slot| slot as i64 + 200)
            .collect(),
    );
    let changed_lookup =
        changed
            .symbols
            .iter()
            .enumerate()
            .fold(Lookup::new(), |lookup, (slot, symbol)| {
                lookup.with(sym(
                    slot as i64 + 200,
                    "poisoned",
                    "poisoned",
                    symbol.kind.as_str(),
                    &changed.path,
                ))
            });
    let changed_row = changed
        .symbols
        .iter()
        .position(|s| s.name == "make")
        .unwrap() as i64
        + 200;
    cold.inputs.insert(
        changed.path.clone(),
        capture(&changed, &changed_ids).unwrap(),
    );
    cold.rebuild(&changed_lookup);
    assert_eq!(
        cold.binding(&consumer.path, bindings[0], false),
        BindingResult::Missing
    );
    assert_eq!(
        cold.binding(&consumer.path, bindings[1], false),
        BindingResult::Bound(changed_row)
    );
    cold.inputs.remove(&provider.path);
    cold.rebuild(&changed_lookup);
    for binding in bindings {
        assert_eq!(
            cold.binding(&consumer.path, binding, false),
            BindingResult::Missing
        );
    }
}
