use super::*;

#[test]
fn ambient_source_units_and_import_origins_survive_contract_filter_and_portable_recapture() {
    let source = "declare module 'provider' { import { Doc } from 'other'; export namespace Nested { export function make(): Doc; } global { interface Catalog { read(): Doc; } } }";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.d.ts");
    std::fs::write(&path, source).unwrap();
    let arena = crate::type_checker::core::types::TypeArena::new();
    let mut parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "ext:ts:pkg/api.d.ts".into(),
            absolute_path: path,
            language: "typescript",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    crate::indexer::contract_filter::reduce_to_contract(&mut parsed);
    let payload = super::super::external_parse_payload::CachedParse::from_parsed(&parsed, &arena);
    let json = serde_json::to_string(&payload).unwrap();
    let payload: super::super::external_parse_payload::CachedParse =
        serde_json::from_str(&json).unwrap();
    let cold_arena = crate::type_checker::core::types::TypeArena::new();
    let mut cold = payload.into_parsed(
        &cold_arena,
        &parsed.path,
        &parsed.content_hash,
        parsed.size,
        parsed.mtime,
    );
    cold.content = Some(source.into());
    restore(&mut cold);
    let mut evidence = Vec::new();
    for file in [&parsed, &cold] {
        let graph = file.flow.lexical.as_ref().unwrap();
        assert_eq!(graph.module.units.len(), 3);
        assert_eq!(
            graph
                .module
                .units
                .iter()
                .map(|unit| unit.parent.0)
                .collect::<Vec<_>>(),
            [0, 1, 1]
        );
        let make = file.symbols.iter().position(|s| s.name == "make").unwrap();
        let export = &graph.module.units[1].exports[0];
        let crate::indexer::lexical::modules::ExportTarget::Local {
            value: Some(binding),
            ..
        } = export.target
        else {
            panic!("{export:?}");
        };
        assert_eq!(graph.symbol_slots[&binding], Some(make));
        assert!(graph
            .module
            .imports
            .keys()
            .all(|binding| graph.module.import_units[binding].0 == 1));
        let globals = graph.globals.as_ref().unwrap();
        assert!(globals.complete);
        assert_eq!(globals.augmentations.len(), 1);
        assert_eq!(globals.augmentations[0].unit, graph.module.units[2].id);
        evidence.push((
            graph
                .module
                .units
                .iter()
                .map(|u| {
                    (
                        u.id.0, u.parent.0, u.scope.0, u.kind, u.range, u.body, u.complete,
                    )
                })
                .collect::<Vec<_>>(),
            graph.module.import_units.clone(),
            graph.symbol_slots[&binding],
        ));
    }
    assert_eq!(evidence[0], evidence[1]);
}

#[test]
fn oversized_contracts_keep_physical_signature_owners_through_portable_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.d.ts");
    let source = format!(
        "{}interface Catalog<T> {{ first(): T; }}",
        " ".repeat(crate::indexer::flow::MAX_FLOW_SOURCE_BYTES + 1)
    );
    std::fs::write(&path, &source).unwrap();
    let arena = crate::type_checker::core::types::TypeArena::new();
    let mut parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "ext:ts:pkg/api.d.ts".into(),
            absolute_path: path,
            language: "typescript",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    crate::indexer::contract_filter::reduce_to_contract(&mut parsed);
    let payload = super::super::external_parse_payload::CachedParse::from_parsed(&parsed, &arena);
    let json = serde_json::to_string(&payload).unwrap();
    let payload: super::super::external_parse_payload::CachedParse =
        serde_json::from_str(&json).unwrap();
    let cold_arena = crate::type_checker::core::types::TypeArena::new();
    let mut cold = payload.into_parsed(
        &cold_arena,
        &parsed.path,
        &parsed.content_hash,
        parsed.size,
        parsed.mtime,
    );
    cold.content = Some(source);
    restore(&mut cold);
    for file in [&parsed, &cold] {
        let graph = file
            .flow
            .lexical
            .as_ref()
            .expect("large contract identity capture");
        let global = &graph.globals.as_ref().unwrap().roots[0];
        assert!(graph.globals.as_ref().unwrap().complete);
        let owner = global.slot.expect("exact surviving physical owner");
        let member = file.symbols.iter().position(|s| s.name == "first").unwrap();
        assert_eq!(file.symbols[member].parent_index, Some(owner));
        assert!(matches!(graph.types.returns.get(&member),
            Some(crate::indexer::lexical::type_syntax::TypeExpr::Parameter { owner: Some(slot), index: 0 }) if *slot == owner));
        assert_eq!(file.symbols.len(), parsed.symbols.len());
    }
}

#[test]
fn trait_contract_slots_survive_filtering_and_portable_cache_hydration() {
    use crate::indexer::namespaces::traits::Owner;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.rs");
    let source = "pub fn outer() { trait Hidden { fn hidden(&self); } struct Discard; impl Hidden for Discard { fn hidden(&self) {} } }
        pub trait Save { fn save(&self); } pub struct Doc; impl Save for Doc { fn save(&self) {} }
        pub fn caller(p:Doc) { p.save(); p.save(); }";
    std::fs::write(&path, source).unwrap();
    let arena = crate::type_checker::core::types::TypeArena::new();
    let mut parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "ext:rust:pkg/api.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let old_slot = parsed
        .symbols
        .iter()
        .position(|s| s.name == "Save")
        .unwrap();
    crate::indexer::contract_filter::reduce_to_contract(&mut parsed);
    assert!(!parsed
        .symbols
        .iter()
        .any(|s| s.name == "Hidden" || s.name == "Discard"));
    let new_slot = parsed
        .symbols
        .iter()
        .position(|s| s.name == "Save")
        .unwrap();
    assert_ne!(
        old_slot, new_slot,
        "fixture must actually filter preceding declaration slots"
    );
    let payload = super::super::external_parse_payload::CachedParse::from_parsed(&parsed, &arena);
    let json = serde_json::to_string(&payload).unwrap();
    let payload: super::super::external_parse_payload::CachedParse =
        serde_json::from_str(&json).unwrap();
    let cold_arena = crate::type_checker::core::types::TypeArena::new();
    let mut cold = payload.into_parsed(
        &cold_arena,
        &parsed.path,
        &parsed.content_hash,
        parsed.size,
        parsed.mtime,
    );
    cold.content = Some(source.into());
    restore(&mut cold);
    for file in [&parsed, &cold] {
        let data = &file.flow.namespaces.as_ref().unwrap().traits;
        let declaration = data
            .headers
            .iter()
            .find(|h| h.owner == Owner::Declaration(new_slot))
            .unwrap();
        assert_eq!(declaration.members.len(), 1);
        assert!(declaration.enabled);
        assert_eq!(
            file.symbols[declaration.members[0]].parent_index,
            Some(new_slot)
        );
        assert_eq!(
            file.symbols[declaration.members[0]].kind,
            crate::types::SymbolKind::Method
        );
        let implementation = data
            .headers
            .iter()
            .find(|h| matches!(h.owner, Owner::Implementation(_)) && h.enabled)
            .unwrap();
        assert_eq!(implementation.members.len(), 1);
        assert_ne!(declaration.members[0], implementation.members[0]);
        assert_eq!(file.symbols[implementation.members[0]].name, "save");
        assert_eq!(data.method_scopes.len(), 2);
        assert_eq!(
            data.method_scopes
                .values()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            1,
            "one source environment shared by both caller sites"
        );
    }
}

#[test]
fn rust_local_binding_ids_and_source_initializer_addresses_survive_metadata_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.rs");
    let source = "struct Doc; impl Doc { fn new() -> Self { Self } fn touch(&self) {} }
        fn f() { let p = Doc::new(); let p = Doc::new(); p.touch(); }";
    std::fs::write(&path, source).unwrap();
    let arena = crate::type_checker::core::types::TypeArena::new();
    let mut file = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "a.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let graph = file.flow.lexical.as_ref().unwrap();
    let slots = graph.symbol_slots.clone();
    let addresses = graph.types.call_initializers.clone();
    let reads = graph.references.clone();
    assert_eq!(addresses.len(), 2);
    let count = file.symbols.len();
    file.flow = Default::default();
    restore(&mut file);
    let graph = file.flow.lexical.as_ref().unwrap();
    assert_eq!(graph.symbol_slots, slots);
    assert_eq!(graph.types.call_initializers, addresses);
    assert_eq!(graph.references, reads);
    assert_eq!(
        file.symbols.len(),
        count,
        "metadata-only restore cannot synthesize new rows"
    );
    crate::indexer::contract_filter::reduce_to_contract(&mut file);
    assert!(!file.symbols.iter().any(|s| s.name == "p"));
    assert!(file
        .flow
        .lexical
        .as_ref()
        .unwrap()
        .types
        .call_initializers
        .is_empty());
}

#[test]
fn reduced_and_cache_equivalent_sources_rebuild_the_same_export_slots() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.ts");
    let source = "export class Result { save(): void {} } function hidden() { const discarded = 1; } export function create(): Result { throw 0; }";
    std::fs::write(&path, source).unwrap();
    let arena = crate::type_checker::core::types::TypeArena::new();
    let mut parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "ext:ts:pkg/api.ts".into(),
            absolute_path: path,
            language: "typescript",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    crate::indexer::contract_filter::reduce_to_contract(&mut parsed);
    let graph = parsed.flow.lexical.as_ref().unwrap();
    let export = graph
        .module
        .exports
        .iter()
        .find(|export| export.name == "create")
        .unwrap();
    let crate::indexer::lexical::modules::ExportTarget::Local {
        value: Some(binding),
        ..
    } = export.target
    else {
        panic!("named export")
    };
    let slot = graph.symbol_slots[&binding].unwrap();
    assert_eq!(parsed.symbols[slot].name, "create");
    assert!(graph.types.returns.contains_key(&slot));
    assert!(parsed.flow.flow_return_lhs.is_empty());
    let prior = graph.symbol_slots.clone();
    parsed.flow = Default::default(); // The external payload's cache-hit shape.
    restore(&mut parsed);
    assert_eq!(parsed.flow.lexical.as_ref().unwrap().symbol_slots, prior);
    assert!(parsed.flow.flow_return_lhs.is_empty());
}

#[test]
fn rust_namespace_contracts_rebind_filtered_slots_after_serialized_cache_hydration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.rs");
    let source = "pub fn outer() { struct Hidden; impl Hidden { fn new() -> Self { Self } } }
        pub mod api { pub struct RealDoc; impl RealDoc { pub fn new() -> Self { Self } } }
        pub use api::RealDoc as AliasDoc;";
    std::fs::write(&path, source).unwrap();
    let arena = crate::type_checker::core::types::TypeArena::new();
    let mut parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "ext:rust:pkg/api.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    crate::indexer::contract_filter::reduce_to_contract(&mut parsed);
    assert!(!parsed.symbols.iter().any(|s| s.name == "Hidden"));
    let payload = super::super::external_parse_payload::CachedParse::from_parsed(&parsed, &arena);
    let json = serde_json::to_string(&payload).unwrap();
    let payload: super::super::external_parse_payload::CachedParse =
        serde_json::from_str(&json).unwrap();
    let cold_arena = crate::type_checker::core::types::TypeArena::new();
    let mut cold = payload.into_parsed(
        &cold_arena,
        &parsed.path,
        &parsed.content_hash,
        parsed.size,
        parsed.mtime,
    );
    cold.content = Some(source.into());
    restore(&mut cold);
    for file in [&parsed, &cold] {
        let data = file.flow.namespaces.as_ref().unwrap();
        assert_eq!(data.units.len(), 2);
        assert!(data.roots.is_empty() && data.selectors.is_empty());
        assert!(file.flow.flow_return_lhs.is_empty());
        assert!(data
            .exports
            .iter()
            .any(|e| data.spelling(e.name) == "AliasDoc"));
        let returns = &file.flow.lexical.as_ref().unwrap().types.returns;
        assert_eq!(returns.len(), 1);
        let (&slot, recipe) = returns.iter().next().unwrap();
        use crate::indexer::lexical::type_syntax::TypeExpr;
        let TypeExpr::Output { inputs, result } = recipe else {
            panic!("source-bound output recipe");
        };
        assert!(inputs.is_empty());
        let TypeExpr::OutputApplication { base, args } = result.as_ref() else {
            panic!("output application");
        };
        assert!(args.is_empty());
        let TypeExpr::Source { usage, .. } = base.as_ref() else {
            panic!("source-bound return recipe")
        };
        let binding = usage.binding;
        assert_eq!(file.symbols[slot].name, "new");
        let crate::indexer::namespaces::Target::Binding(owner_binding) =
            data.bindings[binding.0].targets[0]
        else {
            panic!("return must bind Self by ID")
        };
        let mut target = &data.bindings[owner_binding.0].targets[0];
        let mut visited = std::collections::HashSet::new();
        while let crate::indexer::namespaces::Target::Binding(binding) = target {
            assert!(visited.insert(*binding), "Self must not cycle");
            let [next] = data.bindings[binding.0].targets.as_slice() else {
                panic!("Self must be unambiguous");
            };
            target = next;
        }
        let &crate::indexer::namespaces::Target::Declaration(owner) = target else {
            panic!("Self must carry an exact declaration slot")
        };
        assert_eq!(file.symbols[owner].name, "RealDoc");
        assert_eq!(file.symbols[slot].parent_index, Some(owner));
    }
}

#[test]
fn filtered_and_portable_cached_output_elision_rebinds_to_the_filtered_function_owner() {
    use crate::indexer::resolve::engine::{compilation::Compilation, contract::SymbolLookup};
    use crate::type_checker::core::types::{Type, TypeArena};
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.rs");
    let source = "pub fn outer() { struct Hidden; impl Hidden { fn hidden() {} } }
        pub struct Doc; pub struct Item<T> { pub inner: T } pub type Alias<'a> = Item<&'a Doc>;
        pub fn make(p: &Doc) -> Alias<'_> { loop {} }";
    std::fs::write(&path, source).unwrap();
    let arena = Arc::new(TypeArena::new());
    let mut parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "ext:rust:pkg/api.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    crate::indexer::contract_filter::reduce_to_contract(&mut parsed);
    assert!(!parsed.symbols.iter().any(|s| s.name == "Hidden"));
    let payload = super::super::external_parse_payload::CachedParse::from_parsed(&parsed, &arena);
    let payload: super::super::external_parse_payload::CachedParse =
        serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap();
    let cold_arena = Arc::new(TypeArena::new());
    let mut cold = payload.into_parsed(
        &cold_arena,
        &parsed.path,
        &parsed.content_hash,
        parsed.size,
        parsed.mtime,
    );
    cold.content = Some(source.into());
    restore(&mut cold);
    for (file, arena) in [(parsed, arena), (cold, cold_arena)] {
        let make = file.symbols.iter().position(|s| s.name == "make").unwrap();
        let alias = file.symbols.iter().position(|s| s.name == "Alias").unwrap();
        let files = [file];
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &files,
            "external",
            Some(&arena),
        )
        .unwrap();
        let tree = Compilation::build(&files, &ids, Arc::clone(&arena));
        let owner = ids.row_id(&files[0].path, make).unwrap();
        let info = tree.canonical_type_info(owner).unwrap();
        assert_eq!(info.elided_input_params.len(), 1);
        let Type::Apply { base, args } = arena.get(info.return_type_id.unwrap()) else {
            panic!("output lost its application");
        };
        let Type::Decl { symbol_id, .. } = arena.get(base) else {
            panic!("output lost its owner");
        };
        assert_eq!(symbol_id, ids.row_id(&files[0].path, alias).unwrap());
        assert_eq!(args, [arena.generic_type(info.elided_input_params[0].2)]);
    }
}

#[test]
fn filtered_and_portable_cached_receiver_regions_keep_method_identity_and_argument_positions() {
    use crate::indexer::resolve::engine::{compilation::Compilation, contract::SymbolLookup};
    use crate::type_checker::core::types::{Indirection, Type, TypeArena};
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.rs");
    let source = "pub fn outer() { struct Hidden; impl Hidden { fn hidden() { let x = loop {}; x.make(); } } }
        pub struct Doc; pub struct Item<T> { pub inner: T } pub type Alias<'a> = Item<&'a Doc>;
        pub struct C; impl C { pub fn make(&self, other: &Doc) -> Alias<'_> { loop {} } }
        pub fn caller(p:C, other:&Doc) { p.make(other); }";
    std::fs::write(&path, source).unwrap();
    let arena = Arc::new(TypeArena::new());
    let mut parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "ext:rust:pkg/api.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    crate::indexer::contract_filter::reduce_to_contract(&mut parsed);
    assert!(!parsed.symbols.iter().any(|s| s.name == "Hidden"));
    let payload = super::super::external_parse_payload::CachedParse::from_parsed(&parsed, &arena);
    let payload: super::super::external_parse_payload::CachedParse =
        serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap();
    let cold_arena = Arc::new(TypeArena::new());
    let mut cold = payload.into_parsed(
        &cold_arena,
        &parsed.path,
        &parsed.content_hash,
        parsed.size,
        parsed.mtime,
    );
    cold.content = Some(source.into());
    restore(&mut cold);
    for (file, arena) in [(parsed, arena), (cold, cold_arena)] {
        let make = file.symbols.iter().position(|s| s.name == "make").unwrap();
        let owner = file.symbols.iter().position(|s| s.name == "C").unwrap();
        let caller = file
            .symbols
            .iter()
            .position(|s| s.name == "caller")
            .unwrap();
        let calls = &file.flow.namespaces.as_ref().unwrap().method_calls;
        assert_eq!(
            calls.len(),
            1,
            "filtered local bodies cannot retain stale caller slots"
        );
        assert_eq!(
            calls[&(source.rfind("make(other)").unwrap() as u32)],
            caller
        );
        let files = [file];
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &files,
            "external",
            Some(&arena),
        )
        .unwrap();
        let tree = Compilation::build(&files, &ids, Arc::clone(&arena));
        let info = tree
            .canonical_type_info(ids.row_id(&files[0].path, make).unwrap())
            .unwrap();
        assert_eq!(info.elided_input_params.len(), 2);
        assert_eq!(info.parameter_type_ids.as_ref().unwrap().len(), 1);
        let Type::Indirect {
            kind: Indirection::Reference(region),
            inner,
            ..
        } = arena.get(info.receiver_type_id.unwrap())
        else {
            panic!("receiver");
        };
        assert!(
            matches!(arena.get(inner), Type::Decl { symbol_id, .. } if symbol_id == ids.row_id(&files[0].path, owner).unwrap())
        );
        let Type::Apply { args, .. } = arena.get(info.return_type_id.unwrap()) else {
            panic!("output application");
        };
        assert_eq!(arena.get(args[0]), Type::Region(region));
        let Type::Indirect {
            kind: Indirection::Reference(other),
            ..
        } = arena.get(info.parameter_type_ids.as_ref().unwrap()[0])
        else {
            panic!("ordinary argument");
        };
        assert_ne!(region, other);
    }
}

#[test]
fn explicit_borrow_sources_are_recaptured_after_filtering_and_portable_cache_hydration() {
    use crate::type_checker::core::types::{Mutability, TypeArena};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.rs");
    let source = "pub fn outer() { fn hidden(p:Doc) { Doc::take(&p); } } pub struct Doc;
        pub fn caller(mut p:Doc) { Doc::take(&p); Doc::take(&mut p); Doc::take(&raw const p); }";
    std::fs::write(&path, source).unwrap();
    let arena = TypeArena::new();
    let mut parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "ext:rust:pkg/api.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let raw_payload =
        super::super::external_parse_payload::CachedParse::from_parsed(&parsed, &arena);
    let raw_payload: super::super::external_parse_payload::CachedParse =
        serde_json::from_str(&serde_json::to_string(&raw_payload).unwrap()).unwrap();
    let raw_arena = TypeArena::new();
    let mut raw_cold = raw_payload.into_parsed(
        &raw_arena,
        &parsed.path,
        &parsed.content_hash,
        parsed.size,
        parsed.mtime,
    );
    let borrow_spans = |file: &crate::types::ParsedFile| {
        file.refs
            .iter()
            .filter_map(|r| r.chain.as_ref())
            .flat_map(|c| &c.segments)
            .flat_map(|s| &s.call_args)
            .filter_map(|arg| {
                if let crate::types::CallArg::BorrowAt { span, .. } = arg {
                    Some(*span)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(borrow_spans(&parsed).len(), 3);
    assert_eq!(
        borrow_spans(&parsed),
        borrow_spans(&raw_cold),
        "portable syntax survives without borrowing an arena ID"
    );
    raw_cold.content = Some(source.into());
    restore(&mut raw_cold);
    assert_eq!(
        raw_cold
            .flow
            .namespaces
            .as_ref()
            .unwrap()
            .borrow_sites
            .len(),
        3
    );
    let arguments = parsed
        .flow
        .namespaces
        .as_ref()
        .unwrap()
        .call_arguments
        .clone();
    assert_eq!(arguments.len(), 4);
    assert_eq!(
        raw_cold.flow.namespaces.as_ref().unwrap().call_arguments,
        arguments,
        "semantic operands are rebuilt from source after an unfiltered portable round trip"
    );
    crate::indexer::contract_filter::reduce_to_contract(&mut parsed);
    assert!(!parsed.symbols.iter().any(|s| s.name == "hidden"));
    let cached = super::super::external_parse_payload::CachedParse::from_parsed(&parsed, &arena);
    let cached: super::super::external_parse_payload::CachedParse =
        serde_json::from_str(&serde_json::to_string(&cached).unwrap()).unwrap();
    let cold_arena = TypeArena::new();
    let mut cold = cached.into_parsed(
        &cold_arena,
        &parsed.path,
        &parsed.content_hash,
        parsed.size,
        parsed.mtime,
    );
    cold.content = Some(source.into());
    restore(&mut cold);
    for file in [parsed, cold] {
        let owner = file
            .symbols
            .iter()
            .position(|s| s.name == "caller")
            .unwrap();
        let sites = &file.flow.namespaces.as_ref().unwrap().borrow_sites;
        assert_eq!(
            sites.len(),
            2,
            "filtered nested owners and raw borrows cannot attest regions"
        );
        for (&span, &(slot, mutability)) in sites {
            assert_eq!(slot, owner);
            assert_eq!(
                mutability,
                if &source[span.start as usize..span.end as usize] == "&p" {
                    Mutability::Shared
                } else {
                    Mutability::Mutable
                }
            );
        }
        assert_eq!(file.flow.namespaces.as_ref().unwrap().call_arguments, arguments,
            "source syntax survives filtering, independently of removed body references and caller slots");
        assert!(
            borrow_spans(&file).is_empty(),
            "external contract filtering must not restore discarded body calls"
        );
    }
}

#[test]
fn local_value_recipes_are_recaptured_with_surviving_owner_slots_after_contract_cache_reload() {
    use crate::indexer::lexical::type_syntax::ValueExpr;
    use crate::type_checker::core::types::{Mutability, TypeArena};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.rs");
    let source = "pub struct Doc; pub fn outer() { fn hidden(p:Doc) { let r=&p; } }
        pub fn caller(mut p:Doc) { let r=&mut p; let copied=r; }";
    std::fs::write(&path, source).unwrap();
    let arena = TypeArena::new();
    let mut parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "ext:rust:pkg/api.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let borrows = |file: &crate::types::ParsedFile| {
        file.flow
            .lexical
            .as_ref()
            .unwrap()
            .types
            .values
            .values()
            .filter(|value| matches!(value, ValueExpr::Borrow { .. }))
            .count()
    };
    assert_eq!(borrows(&parsed), 2);
    crate::indexer::contract_filter::reduce_to_contract(&mut parsed);
    let cached = super::super::external_parse_payload::CachedParse::from_parsed(&parsed, &arena);
    let cached: super::super::external_parse_payload::CachedParse =
        serde_json::from_str(&serde_json::to_string(&cached).unwrap()).unwrap();
    let cold_arena = TypeArena::new();
    let mut cold = cached.into_parsed(
        &cold_arena,
        &parsed.path,
        &parsed.content_hash,
        parsed.size,
        parsed.mtime,
    );
    cold.content = Some(source.into());
    restore(&mut cold);
    for file in [parsed, cold] {
        assert!(!file.symbols.iter().any(|s| s.name == "hidden"));
        assert_eq!(
            borrows(&file),
            1,
            "a filtered nested owner cannot survive as a stale slot"
        );
        let owner = file
            .symbols
            .iter()
            .position(|s| s.name == "caller")
            .unwrap();
        let graph = file.flow.lexical.as_ref().unwrap();
        let binding = |token: &str| graph.declaration_starts[&(source.find(token).unwrap() as u32)];
        assert_eq!(graph.types.values[&binding("r=&p")], ValueExpr::Unknown);
        let ValueExpr::Borrow {
            owner: actual,
            span,
            mutability,
            operand,
        } = &graph.types.values[&binding("r=&mut")]
        else {
            panic!("source local borrow");
        };
        assert_eq!(*actual, owner);
        assert_eq!(*mutability, Mutability::Mutable);
        assert_eq!(&source[span.start as usize..span.end as usize], "&mut p");
        assert!(matches!(operand.as_ref(), ValueExpr::Read { .. }));
        assert_eq!(
            graph.types.values[&binding("copied=")],
            ValueExpr::Read {
                binding: binding("r=&mut"),
                byte: source.rfind("=r;").unwrap() as u32 + 1
            }
        );
    }
}

#[test]
fn source_place_recipes_survive_filtered_portable_cache_recapture() {
    use crate::type_checker::core::types::TypeArena;
    let source="pub struct Doc; pub struct Holder { pub r#item:Doc } pub fn caller(p:Holder) { let local=&p.item; keep(&p.r#item); keep(*&p.item); } pub fn keep<T>(x:T)->T {x}";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.rs");
    std::fs::write(&path, source).unwrap();
    let arena = TypeArena::new();
    let mut parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "ext:rust:pkg/api.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    crate::indexer::contract_filter::reduce_to_contract(&mut parsed);
    let cached = super::super::external_parse_payload::CachedParse::from_parsed(&parsed, &arena);
    let cached: super::super::external_parse_payload::CachedParse =
        serde_json::from_str(&serde_json::to_string(&cached).unwrap()).unwrap();
    let mut cold = cached.into_parsed(
        &TypeArena::new(),
        &parsed.path,
        &parsed.content_hash,
        parsed.size,
        parsed.mtime,
    );
    cold.content = Some(source.into());
    restore(&mut cold);
    let expected = parsed.flow.lexical.as_ref().unwrap();
    let actual = cold.flow.lexical.as_ref().unwrap();
    assert_eq!(expected.types.expressions.len(), 4);
    assert_eq!(actual.types.expressions, expected.types.expressions);
    assert_eq!(actual.types.values, expected.types.values);
    assert_eq!(
        cold.flow.namespaces.as_ref().unwrap().call_arguments,
        parsed.flow.namespaces.as_ref().unwrap().call_arguments
    );
    assert!(cold
        .symbols
        .iter()
        .any(|s| s.qualified_name == "Holder.item"));
}

#[test]
fn pattern_payload_fields_and_recipes_survive_filtered_portable_cache_recapture() {
    use crate::type_checker::core::types::TypeArena;
    let source="pub enum E<T> { Item(T), Named { item:T } } pub fn caller(p:E<Doc>) { match p { E::Item(value) => value.touch(), E::Named { item:value } => value.touch() } } pub struct Doc; impl Doc { pub fn touch(&self) {} }";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.rs");
    std::fs::write(&path, source).unwrap();
    let arena = TypeArena::new();
    let mut parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "ext:rust:pkg/api.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    crate::indexer::contract_filter::reduce_to_contract(&mut parsed);
    let cached = super::super::external_parse_payload::CachedParse::from_parsed(&parsed, &arena);
    let cached: super::super::external_parse_payload::CachedParse =
        serde_json::from_str(&serde_json::to_string(&cached).unwrap()).unwrap();
    let mut cold = cached.into_parsed(
        &TypeArena::new(),
        &parsed.path,
        &parsed.content_hash,
        parsed.size,
        parsed.mtime,
    );
    cold.content = Some(source.into());
    restore(&mut cold);
    let expected = parsed.flow.lexical.as_ref().unwrap();
    let actual = cold.flow.lexical.as_ref().unwrap();
    assert_eq!(expected.types.pattern_heads.len(), 2);
    assert_eq!(actual.types.pattern_heads.len(), 2);
    assert_eq!(actual.types.values, expected.types.values);
    for name in ["E.Item.0", "E.Named.item"] {
        let slot = cold
            .symbols
            .iter()
            .position(|s| s.qualified_name == name)
            .unwrap();
        assert!(actual.types.fields.contains_key(&slot));
        assert_eq!(
            cold.symbols[cold.symbols[slot].parent_index.unwrap()].kind,
            crate::types::SymbolKind::EnumMember
        );
    }
}
