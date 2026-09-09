use super::super::super::call_arguments as arguments;
use super::super::super::merge_proof::tests::{context, owner, parse};
use super::*;
use crate::indexer::resolve::engine::contract::FlowCacheLookup;
use crate::indexer::symbol_ids::SymbolIds;
use std::{collections::HashSet, sync::Arc};

#[test]
fn readonly_array_constructor_reaches_source_result_type() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("arrays.d.ts", "interface Array<T> { [n: number]: T; length: number } interface ReadonlyArray<T> { readonly [n: number]: T; readonly length: number }"),
        ("main.ts", "export {}; interface Payload { touch(): void } interface Result<T> { read(): T } interface Factory { new<T>(items: readonly T[] | null): Result<T> } declare const Build: Factory; declare const items: Payload[]; class Holder { value = new Build(items) } declare const expected: Result<Payload>;")]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&files.iter().collect::<Vec<_>>())),
        &HashSet::new(),
    );
    let lookup = tree.program_lookup("main.ts").unwrap();
    let expected = lookup
        .field_type_id_of(owner(&ids, &files[1], "expected"))
        .unwrap();
    assert_eq!(
        lookup.field_type_id_of(owner(&ids, &files[1], "value")),
        Some(expected)
    );
}

#[test]
#[ignore = "manual provenance probe requiring the frozen Query Core source manifest"]
fn retained_query_core_constructor_provenance() {
    use crate::indexer::lexical::globals::member_surface::Kind;
    use crate::indexer::{
        parse_file::parse_file_with_arena,
        programs::{Program, ProgramSource, SourceScope},
    };
    use crate::resolution_oracle::project::ProjectManifest;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../resolution-documents/2026-09-07-query-core-configured-compiler-manifest.json");
    let manifest: ProjectManifest = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    manifest.verify_inputs().unwrap();
    let arena = Arc::new(TypeArena::new());
    let registry = crate::languages::default_registry();
    let files: Vec<_> = manifest
        .files
        .iter()
        .map(|file| {
            parse_file_with_arena(
                &crate::walker::WalkedFile {
                    relative_path: file.index_path.clone(),
                    absolute_path: file.path.clone(),
                    language: "typescript",
                },
                &registry,
                &arena,
            )
            .unwrap()
        })
        .collect();
    assert_eq!(manifest.compiler_options["strict"], true);
    assert!(
        manifest
            .compiler_options
            .get("strictFunctionTypes")
            .is_none()
            && manifest.compiler_options.get("strictNullChecks").is_none()
    );
    let supplemental: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../resolution-documents/2026-09-09-query-core-difference-compiler-signatures.json",
        ))
        .unwrap(),
    )
    .unwrap();
    let config = crate::indexer::resolve::ProjectContext {
        programs: Some(vec![Program {
            key: "retained-provenance".into(),
            fingerprint: "retained-provenance".into(),
            complete: true,
            callable_policy: Some(crate::indexer::programs::CallablePolicy {
                strict_parameters: true,
                strict_nulls: true,
                bivariant_methods: Some(true),
            }),
            source_binding_order: Some(
                supplemental["source_binding_order"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| {
                        manifest
                            .files
                            .iter()
                            .find(|f| f.id.0 as u64 == id.as_u64().unwrap())
                            .unwrap()
                            .index_path
                            .clone()
                    })
                    .collect(),
            ),
            compiler_intrinsics:
                crate::resolution_oracle::compiler_intrinsic_policy::typescript_options(
                    &manifest.compiler_options,
                ),
            sources: manifest
                .files
                .iter()
                .map(|file| ProgramSource {
                    path: file.index_path.clone(),
                    content_hash: file.sha256.clone(),
                    scope: file.source_scope.unwrap_or(SourceScope::Unknown),
                })
                .collect(),
        }]),
        ..Default::default()
    };
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    let file = files
        .iter()
        .find(|f| f.path.ends_with("/queriesObserver.ts"))
        .unwrap();
    let source = file.content.as_deref().unwrap();
    let input = program_types::capture(file, &ids, &tree, &arena);
    let start = source.find("excludeSet =").unwrap() as u32;
    let initializer = input
        .initializers
        .iter()
        .find(|i| i.signature.0.start == start)
        .unwrap();
    let lookup = tree.program_lookup(&file.path).unwrap();
    let describe = |ty: TypeId| (ty, arena.get(ty), arena.format_type(ty));
    eprintln!(
        "initializer {:?} result {:?}",
        initializer.signature,
        lookup
            .source
            .unwrap()
            .initializers
            .get(&initializer.signature)
            .copied()
            .flatten()
            .map(describe)
    );
    {
        use crate::indexer::resolve::engine::{
            contract::{FileContext, SymbolLookup},
            file_lookup::FileLookup,
            semantic_model::{SemanticModel, SolveOutcome},
            testkit,
        };
        let labels: Vec<_> = supplemental["signatures"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|label| label["kind"] == "call")
            .collect();
        assert_eq!(labels.len(), 2);
        let file_lookup = FileLookup::for_file(&tree, file, &ids);
        let graph = file.flow.lexical.as_ref().unwrap();
        let receiver_start = source.find("array1.filter").unwrap() as u32;
        file_lookup.set_cursor(receiver_start);
        let receiver = file_lookup
            .local_type_id("array1")
            .expect("source parameter receiver type");
        eprintln!(
            "retained local excludeSet {:?}",
            file_lookup.local_type_id("excludeSet").map(describe)
        );
        assert_eq!(
            file_lookup.local_type_id("excludeSet"),
            lookup
                .source
                .unwrap()
                .initializers
                .get(&initializer.signature)
                .copied()
                .flatten()
        );
        let selector = receiver_start + 7;
        let args = file_lookup
            .source_call_arguments(selector)
            .unwrap()
            .unwrap();
        let args = crate::indexer::resolve::engine::arg_types::resolve_arg_types(
            &file_lookup,
            &arena,
            args,
        );
        let result = file_lookup.overloaded_call(receiver, selector, &args, &[]);
        eprintln!(
            "retained filter receiver {:?}; selection {:?}",
            describe(receiver),
            result.as_ref().map(|r| r.as_ref().map(|c| (
                c.selected,
                c.return_type,
                c.parameters.clone()
            )))
        );
        let call = result
            .unwrap()
            .expect("retained compiler-selected filter signature");
        let origin = &call.origins[call.selected];
        let expected = &labels[0]["selected"];
        let provider = manifest
            .files
            .iter()
            .find(|f| f.id.0 as u64 == expected["file"].as_u64().unwrap())
            .unwrap();
        assert_eq!(
            origin.span.start as u64,
            expected["start"].as_u64().unwrap()
        );
        assert_eq!(
            file_lookup
                .symbol_by_id(origin.declaration.unwrap())
                .unwrap()
                .file_path
                .as_ref(),
            provider.index_path
        );
        assert_eq!(
            call.return_type, receiver,
            "filter must retain the caller's Array<T> identity"
        );
        for callback in graph
            .globals
            .as_ref()
            .unwrap()
            .calls
            .callbacks
            .values()
            .filter(|c| {
                c.signature.start >= receiver_start && c.signature.end < receiver_start + 60
            })
        {
            eprintln!("retained callback {:?}", callback.body);
        }
        let context = FileContext {
            file_path: file.path.clone(),
            language: "typescript".into(),
            imports: vec![],
            file_namespace: None,
        };
        let model = SemanticModel::production();
        let mut checked = HashSet::new();
        for reference in file
            .refs
            .iter()
            .filter(|r| r.kind == crate::types::EdgeKind::Calls)
        {
            let terminal = reference
                .chain
                .as_ref()
                .and_then(|chain| chain.segments.last())
                .map(|s| s.byte_offset);
            let label = match terminal {
                Some(byte) if byte == selector => labels[0],
                Some(byte) if byte == source.find("excludeSet.has").unwrap() as u32 + 11 => {
                    labels[1]
                }
                _ => continue,
            };
            let expected = &label["selected"];
            let provider = manifest
                .files
                .iter()
                .find(|f| f.id.0 as u64 == expected["file"].as_u64().unwrap())
                .unwrap();
            let provider = files
                .iter()
                .find(|f| f.path == provider.index_path)
                .unwrap();
            let prefix =
                &provider.content.as_ref().unwrap()[..expected["start"].as_u64().unwrap() as usize];
            let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
            let col = prefix.rsplit('\n').next().unwrap().len() as u32;
            let slot = provider
                .symbols
                .iter()
                .position(|s| s.start_line == line && s.start_col == col)
                .unwrap();
            let mut site = testkit::ref_ctx(
                reference,
                &file.symbols[reference.source_symbol_index],
                vec![],
            );
            site.source_symbol_id = ids.row_id(&file.path, reference.source_symbol_index);
            file_lookup.set_cursor(reference.byte_offset);
            let SolveOutcome::Resolved(info) = model.get_symbol_info(
                &site,
                &context,
                &file_lookup,
                &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
            ) else {
                panic!("retained constructor/filter/has cascade stopped at {terminal:?}");
            };
            assert_eq!(
                Some(info.target_symbol_id),
                ids.row_id(&provider.path, slot)
            );
            checked.insert(terminal);
        }
        assert_eq!(
            checked.len(),
            2,
            "exact compiler filter and has declarations"
        );
    }
    let Expression::Construct {
        callee,
        arguments: operands,
        types,
    } = &initializer.expression
    else {
        panic!("source construction missing")
    };
    assert!(types.is_empty());
    let callee = lookup
        .source_value_type(*callee)
        .flatten()
        .expect("source runtime constructor value");
    eprintln!("constructor value {:?}", describe(callee));
    let actual: Vec<_> = operands
        .iter()
        .map(|operand| {
            let Expression::Read(site) = operand else {
                panic!("expected source argument read")
            };
            let ty = lookup
                .source_value_type(*site)
                .flatten()
                .expect("source argument annotation");
            eprintln!("argument {site:?}: {:?}", describe(ty));
            ty
        })
        .collect();
    let owner = head_decl_id(&arena, callee).unwrap();
    let relation = super::super::super::merge_proof::types::Relation {
        lookup: &lookup,
        arena: &arena,
    };
    let mut count = 0;
    for provider in &files {
        let inputs = program_types::capture(provider, &ids, &tree, &arena);
        let provider_lookup = tree.program_lookup(&provider.path).unwrap();
        // Source-location diagnostics only: these spellings never participate
        // in engine resolution. Inspect the declarations supplying the return
        // type of the already-ID-bound computed iterator member.
        for (slot, symbol) in provider.symbols.iter().enumerate().filter(|(_, s)| {
            matches!(
                s.name.as_str(),
                "ArrayIterator" | "IteratorObject" | "BuiltinIteratorReturn"
            )
        }) {
            let row = ids.row_id(&provider.path, slot).unwrap();
            eprintln!(
                "protocol provider {} {} row={row} selected={}",
                provider.path,
                symbol.name,
                provider_lookup.symbol_by_id(row).is_some()
            );
            for part in inputs.interfaces.iter().filter(|part| part.owner == row) {
                for base in part.bases.iter().flatten() {
                    let ty = base.materialize(&provider_lookup, &arena, &provider.path);
                    eprintln!(
                        " protocol base {base:?}: {:?}; canonical={:?}",
                        describe(ty),
                        relation.canonical(ty, 0)
                    );
                }
            }
            for signature in inputs
                .signatures
                .iter()
                .filter(|signature| signature.declaration == row)
            {
                eprintln!(" protocol recipe {signature:?}");
                if let Some(body) = provider_lookup
                    .canonical_type_info(row)
                    .and_then(|info| info.lexical_alias.as_ref())
                    .and_then(|alias| alias.instantiate(&arena, &[]))
                {
                    eprintln!(
                        " protocol alias body {:?}; canonical={:?}",
                        describe(body),
                        relation.canonical(body, 0)
                    );
                }
            }
        }
        for interface in inputs
            .interfaces
            .iter()
            .filter(|i| lookup.canonical_decl_id(i.owner) == owner)
        {
            for member in interface
                .surface
                .iter()
                .flatten()
                .filter(|m| m.kind == Kind::Construct)
            {
                let signature = provider_lookup.signature(SignatureId(member.span)).unwrap();
                let applied = arguments::applicable(&relation, signature, &actual, &[])
                    .map(|applied| applied.map(|a| describe(a.result)));
                eprintln!(
                    "candidate {} {:?}: parameters {:?}; result {:?}; applicability {applied:?}",
                    provider.path,
                    member.span,
                    signature
                        .parameters
                        .iter()
                        .copied()
                        .map(describe)
                        .collect::<Vec<_>>(),
                    signature.result.map(describe)
                );
                if applied.is_none() {
                    trace_constructor_protocol(&lookup, &arena, actual[0], 0);
                    for &parameter in &signature.parameters {
                        let parts = match arena.get(parameter) {
                            Type::Union(parts) => parts,
                            _ => vec![parameter],
                        };
                        for part in parts {
                            trace_constructor_protocol(&lookup, &arena, part, 0);
                        }
                    }
                }
                count += 1;
            }
        }
    }
    assert!(
        count > 0,
        "the probe must inspect real constructor signatures"
    );
    let check_constructor = |tree: &Compilation| {
        let lookup = tree.program_lookup(&file.path).unwrap();
        let source = lookup.source.unwrap();
        let call = source
            .constructor_calls
            .get(&initializer.signature)
            .and_then(Option::as_ref)
            .expect("retained constructor evidence");
        let origin = call.origins[call
            .selected
            .expect("compiler-attested constructor selection")]
        .as_ref()
        .unwrap();
        let expected = &supplemental["signatures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["kind"] == "construct")
            .unwrap()["selected"];
        let provider = manifest
            .files
            .iter()
            .find(|f| f.id.0 as u64 == expected["file"].as_u64().unwrap())
            .unwrap();
        assert_eq!(
            Some(origin.source),
            tree.program_lookup(&provider.index_path)
                .unwrap()
                .source
                .unwrap()
                .identity
        );
        assert_eq!(
            origin.span.start as u64,
            expected["start"].as_u64().unwrap()
        );
        assert_eq!(
            Some(call.return_type),
            source
                .initializers
                .get(&initializer.signature)
                .copied()
                .flatten()
        );
    };
    check_constructor(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &Default::default(), restored);
    cold.ingest_from_db(db.conn());
    check_constructor(&cold);
}

// Diagnostic only: follow source-owned protocol members and instantiated
// heritage without asserting that matching keys prove structural compatibility.
fn trace_constructor_protocol(
    lookup: &super::super::super::Lookup,
    arena: &TypeArena,
    ty: TypeId,
    depth: usize,
) {
    if depth > 3 {
        return;
    }
    let Some(owner) = head_decl_id(arena, ty) else {
        return;
    };
    let Some(surface) = lookup.nominal_surface(owner) else {
        return;
    };
    let relation = super::super::super::merge_proof::types::Relation { lookup, arena };
    let info = lookup.canonical_type_info(owner).unwrap();
    let args = match arena.get(ty) {
        Type::Apply { args, .. } => args,
        _ => vec![],
    };
    let mut bindings: FxHashMap<_, _> = info.generic_param_ids.iter().copied().zip(args).collect();
    for (&parameter, default) in info
        .generic_param_ids
        .iter()
        .zip(&info.generic_param_default_ids)
    {
        if !bindings.contains_key(&parameter) {
            if let Some(default) = default {
                bindings.insert(
                    parameter,
                    generic_return::substitute(arena, *default, &bindings),
                );
            }
        }
    }
    eprintln!(
        "protocol depth={depth} owner={owner} type={} canonical={:?} incomplete={}",
        arena.format_type(ty),
        relation.canonical(ty, 0),
        surface.incomplete
    );
    for member in &surface.members {
        if depth == 0 && !matches!(arena.get(member.property.key), Type::UniqueSymbol(_)) {
            continue;
        }
        let result = member
            .signature
            .result
            .map(|ty| generic_return::substitute(arena, ty, &bindings));
        eprintln!(
            " protocol member {:?} key={:?} result={:?} canonical={:?} rest={}",
            member.origin.signature,
            arena.get(member.property.key),
            result.map(|ty| arena.format_type(ty)),
            result.and_then(|ty| relation.canonical(ty, 0)),
            member.signature.syntax.parameters.iter().any(|p| p.rest)
        );
        if depth == 0 {
            if let Some(result) = result {
                trace_constructor_protocol(lookup, arena, result, depth + 1);
            }
        }
    }
    for &base in &surface.bases {
        trace_constructor_protocol(
            lookup,
            arena,
            generic_return::substitute(arena, base, &bindings),
            depth + 1,
        );
    }
}

#[test]
fn nullable_constructor_inference_flips_portable_poisoned_local_and_inline_cascades() {
    check_poisoned_constructor_cascade(false);
}

#[test]
fn readonly_array_constructor_flips_portable_poisoned_local_and_inline_cascades() {
    check_poisoned_constructor_cascade(true);
}

fn check_poisoned_constructor_cascade(array: bool) {
    use crate::indexer::resolve::engine::{
        contract::FileContext,
        file_lookup::FileLookup,
        semantic_model::{SemanticModel, SolveOutcome},
        testkit,
    };
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    use crate::types::{CallArg, EdgeKind};
    let cases: Vec<serde_json::Value> = serde_json::from_str(if array {
        include_str!("../../../resolution_oracle/array_initializer_fixtures.json")
    } else {
        include_str!("../../../resolution_oracle/initializer_fixtures.json")
    })
    .unwrap();
    let source = cases
        .iter()
        .find(|c| {
            c["name"]
                == if array {
                    "mutable array infers readonly constructor element"
                } else {
                    "nullable constructor return cascade"
                }
        })
        .unwrap()["source"]
        .as_str()
        .unwrap();
    let source = if array {
        format!("interface Array<T> {{ [n: number]: T; length: number }} interface ReadonlyArray<T> {{ readonly [n: number]: T; readonly length: number }} {source} function run(holder: Holder) {{ const item = holder.value.read(); item.touch(); holder.value.read().touch(); }}")
    } else {
        source.to_owned()
    };
    let split = source.find("function run").unwrap();
    let provider = &source[..split];
    let consumer = &source[split..];
    let original = TypeArena::new();
    let parsed = parse(&original, &[("provider.ts", provider)]);
    let payload = serde_json::to_string(&CachedParse::from_parsed(&parsed[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    for n in 0..32 {
        arena.intern(Type::Literal(
            crate::type_checker::core::types::LitValue::Int(n),
        ));
    }
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut provider_file = cached.into_parsed(
        &arena,
        "provider.ts",
        &parsed[0].content_hash,
        parsed[0].size,
        None,
    );
    provider_file.content = Some(provider.into());
    reduce_to_contract(&mut provider_file);
    let file = parse(&arena, &[("main.ts", consumer)]).remove(0);
    let mut files = vec![provider_file, file];
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let target = owner(&ids, &files[0], "touch");
    let holder_owner = owner(&ids, &files[0], "Holder");
    let value_row = owner(&ids, &files[0], "value");
    let result_owner = owner(&ids, &files[0], "Result");
    let read_row = owner(&ids, &files[0], "read");
    for reference in &mut files[1].refs {
        reference.call_args = vec![CallArg::Ident("poisoned".into())];
        for segment in reference.chain.iter_mut().flat_map(|c| &mut c.segments) {
            segment.name = "poisoned".into();
            segment.call_args.clear();
        }
    }
    for file in &mut files {
        for symbol in &mut file.symbols {
            symbol.name = "poisoned".into();
            symbol.qualified_name = "poisoned.display".into();
            symbol.signature = None;
        }
    }
    let file = &files[1];
    let check = |tree: &Compilation| {
        let lookup = FileLookup::for_file(tree, file, &ids);
        let program = tree.program_lookup("main.ts").unwrap();
        for (owner, spelling, row) in [
            (holder_owner, "value", value_row),
            (result_owner, "read", read_row),
        ] {
            let name = program.view.members.name(spelling).unwrap();
            assert_eq!(
                program.view.members.candidates(owner, name),
                [row],
                "source member {spelling} lost its navigation ID"
            );
            if let Some(poisoned) = program.view.members.name("poisoned") {
                assert!(
                    !program
                        .view
                        .members
                        .candidates(owner, poisoned)
                        .contains(&row),
                    "display spelling must not retain a member alias"
                );
            }
        }
        let context = FileContext {
            file_path: "main.ts".into(),
            language: "typescript".into(),
            imports: vec![],
            file_namespace: None,
        };
        let model = SemanticModel::production();
        let mut downstream = HashSet::new();
        for (index, reference) in file
            .refs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == EdgeKind::Calls)
        {
            let mut site = testkit::ref_ctx(
                reference,
                &file.symbols[reference.source_symbol_index],
                vec![],
            );
            site.source_symbol_id = ids.row_id("main.ts", reference.source_symbol_index);
            lookup.set_cursor(reference.byte_offset);
            let info = match model.get_symbol_info(
                &site,
                &context,
                &lookup,
                &crate::languages::typescript::profile::TYPESCRIPT_PROFILE,
            ) {
                SolveOutcome::Resolved(info) => info,
                SolveOutcome::Unresolved(cause) => panic!(
                    "nullable constructor cascade stopped at {}: {cause:?}",
                    reference.byte_offset
                ),
                SolveOutcome::Drained => panic!(
                    "nullable constructor cascade drained at {}",
                    reference.byte_offset
                ),
            };
            if let Some(ty) = info.resolved_yield_type {
                lookup.record_rhs_type(index, "poisoned", ty);
            }
            if info.target_symbol_id == target {
                downstream.insert(
                    reference
                        .chain
                        .as_ref()
                        .unwrap()
                        .segments
                        .last()
                        .unwrap()
                        .byte_offset,
                );
            }
        }
        assert_eq!(
            downstream.len(),
            2,
            "both local and inline generic returns must reach the source Payload member"
        );
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0], &files[1]])),
        &HashSet::new(),
    );
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &Default::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
}

#[test]
fn compiler_checked_constructor_results_and_barriers_fresh_and_cold() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../resolution_oracle/initializer_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        let arena = Arc::new(TypeArena::new());
        let files = parse(
            &arena,
            &[(
                "main.ts",
                &format!("export {{}}; {}", case["source"].as_str().unwrap()),
            )],
        );
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &files,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let field = owner(&ids, &files[0], "value");
        let expected = case["supported"]
            .as_bool()
            .unwrap()
            .then(|| owner(&ids, &files[0], "expected"));
        let tree = Compilation::build_with_context(
            &files,
            &ids,
            Arc::clone(&arena),
            Some(&context(&[&files[0]])),
            &HashSet::new(),
        );
        let check = |tree: &Compilation| {
            let lookup = tree.program_lookup("main.ts").unwrap();
            let actual = lookup.field_type_id_of(field);
            if let Some(expected) = expected {
                let expected = lookup
                    .field_type_id_of(expected)
                    .expect("expected source annotation");
                assert_eq!(
                    actual,
                    Some(expected),
                    "{}: actual {:?}, expected {}",
                    case["name"],
                    actual.map(|ty| tree.type_arena().unwrap().format_type(ty)),
                    tree.type_arena().unwrap().format_type(expected)
                );
            } else {
                assert!(
                    actual.is_none_or(|ty| matches!(
                        tree.type_arena().unwrap().get(ty),
                        Type::Unknown
                    )),
                    "{}: fabricated initializer",
                    case["name"]
                );
            }
        };
        check(&tree);
        tree.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &Default::default(), restored);
        cold.ingest_from_db(db.conn());
        check(&cold);
    }
}

#[test]
fn constructor_provider_retarget_edit_and_deletion_rebind_unchanged_consumers() {
    for inferred in [false, true] {
        let arena = Arc::new(TypeArena::new());
        let provider = if inferred {
            "export interface Result<T> { read(): T } interface Factory { new<T>(input: T | null): Result<T> } export declare const Build: Factory;"
        } else {
            "export interface Result<T> { read(): T } interface Factory { new<T>(): Result<T> } export declare const Build: Factory;"
        };
        let consumer = if inferred {
            "import { Build } from './barrel'; class Holder { value = new Build('text') }"
        } else {
            "import { Build } from './barrel'; class Holder { value = new Build<string>() }"
        };
        let files = parse(
            &arena,
            &[
                ("left.ts", provider),
                ("right.ts", provider),
                ("barrel.ts", "export { Build } from './left';"),
                ("main.ts", consumer),
            ],
        );
        let db = crate::Database::open_in_memory().unwrap();
        let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &files,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let field = owner(&ids, &files[3], "value");
        let left = owner(&ids, &files[0], "Result");
        let right = owner(&ids, &files[1], "Result");
        assert_ne!(left, right);
        let check = |tree: &Compilation, wanted: Option<i64>| {
            let lookup = tree.program_lookup("main.ts").unwrap();
            let arena = tree.type_arena().unwrap();
            let ty = lookup.field_type_id_of(field);
            assert_eq!(ty.and_then(|ty| head_decl_id(arena, ty)), wanted);
            if wanted.is_some() {
                let Type::Apply { args, .. } = arena.get(ty.unwrap()) else {
                    panic!("generic result was lost")
                };
                assert_eq!(
                    args,
                    [arena.intern(Type::Intrinsic(
                        crate::type_checker::core::types::Intrinsic::String
                    ))]
                );
            }
        };
        let tree = Compilation::build_with_context(
            &files,
            &ids,
            Arc::clone(&arena),
            Some(&context(&files.iter().collect::<Vec<_>>())),
            &HashSet::new(),
        );
        check(&tree, Some(left));
        tree.persist_type_info(db.conn()).unwrap();
        let changed = parse(&arena, &[("barrel.ts", "export { Build } from './right';")]);
        let (_, changed_ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &changed,
            "internal",
            Some(&arena),
        )
        .unwrap();
        let selected = context(&[&files[0], &files[1], &changed[0], &files[3]]);
        let mut retargeted = Compilation::build_with_context(
            &changed,
            &changed_ids,
            Arc::clone(&arena),
            Some(&selected),
            &HashSet::new(),
        );
        retargeted.ingest_from_db(db.conn());
        check(&retargeted, Some(right));
        retargeted.persist_type_info(db.conn()).unwrap();
        let restored = Arc::new(TypeArena::new());
        restored.restore_snapshot(&arena.serialize_snapshot());
        let mut cold = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&restored));
        cold.ingest_from_db(db.conn());
        check(&cold, Some(right));
        let invalid = parse(&restored, &[("right.ts", "export interface Result<T> { read(): T } interface Factory { new<T>(required: number): Result<T> } export declare const Build: Factory;")]);
        let (_, invalid_ids) = crate::indexer::write::write_parsed_files_with_origin(
            &db,
            &invalid,
            "internal",
            Some(&restored),
        )
        .unwrap();
        let selected = context(&[&files[0], &invalid[0], &changed[0], &files[3]]);
        let mut edited = Compilation::build_with_context(
            &invalid,
            &invalid_ids,
            Arc::clone(&restored),
            Some(&selected),
            &HashSet::new(),
        );
        edited.ingest_from_db(db.conn());
        check(&edited, None);
        edited.persist_type_info(db.conn()).unwrap();
        db.conn()
            .execute("DELETE FROM files WHERE path='right.ts'", [])
            .unwrap();
        let remaining = context(&[&files[0], &changed[0], &files[3]]);
        let mut deleted = Compilation::build_with_context(
            &[],
            &SymbolIds::default(),
            Arc::clone(&restored),
            Some(&remaining),
            &HashSet::new(),
        );
        deleted.ingest_from_db(db.conn());
        check(&deleted, None);
        deleted.persist_type_info(db.conn()).unwrap();
        let final_arena = Arc::new(TypeArena::new());
        final_arena.restore_snapshot(&restored.serialize_snapshot());
        let mut final_cold = Compilation::build(&[], &SymbolIds::default(), final_arena);
        final_cold.ingest_from_db(db.conn());
        check(&final_cold, None);
    }
}

#[test]
fn rowless_field_initializers_survive_portable_poisoned_and_cold_sources() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let original = TypeArena::new();
    let source = "function discard() {} interface Result<T> { read(): T } interface Factory { new<T>(): Result<T> } declare const Build: Factory; class Holder { value = new Build<string>() } declare const holder: Holder; type Picked = typeof holder.value; declare const expected: Result<string>;";
    let mut files = parse(&original, &[("main.ts", source)]);
    let discarded = files[0]
        .symbols
        .iter()
        .position(|s| s.name == "discard")
        .unwrap();
    files[0]
        .symbols
        .iter_mut()
        .find(|s| s.name == "value")
        .unwrap()
        .parent_index = Some(discarded);
    reduce_to_contract(&mut files[0]);
    assert!(!files[0].symbols.iter().any(|s| s.name == "value"));
    let payload = serde_json::to_string(&CachedParse::from_parsed(&files[0], &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    for value in 0..40 {
        arena.intern(Type::Literal(
            crate::type_checker::core::types::LitValue::Int(value),
        ));
    }
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut file = cached.into_parsed(
        &arena,
        "main.ts",
        &files[0].content_hash,
        files[0].size,
        None,
    );
    file.content = Some(source.into());
    reduce_to_contract(&mut file);
    assert!(file
        .flow
        .lexical
        .as_ref()
        .unwrap()
        .types
        .initializers
        .iter()
        .any(|i| i.declaration.is_none()));
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&file),
        "external",
        Some(&arena),
    )
    .unwrap();
    let expected = owner(&ids, &file, "expected");
    let tree = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&file])),
        &HashSet::new(),
    );
    let start = source.find("typeof holder.value").unwrap() as u32;
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("main.ts").unwrap();
        assert_eq!(
            lookup
                .source_value_type(SourceSpan {
                    start,
                    end: start + 19
                })
                .flatten(),
            lookup.field_type_id_of(expected)
        );
        assert!(lookup.field_type_id_of(expected).is_some());
    };
    check(&tree);
    let before = serde_json::to_value(program_types::capture(&file, &ids, &tree, &arena)).unwrap();
    for symbol in &mut file.symbols {
        symbol.name = "poison".into();
        symbol.qualified_name = "poison.display".into();
        symbol.signature = None;
    }
    assert_eq!(
        serde_json::to_value(program_types::capture(&file, &ids, &tree, &arena)).unwrap(),
        before
    );
    let poisoned = Compilation::build_with_context(
        std::slice::from_ref(&file),
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&file])),
        &HashSet::new(),
    );
    check(&poisoned);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
}

#[test]
fn cyclic_constructor_result_does_not_publish_a_provisional_field_type() {
    let arena = Arc::new(TypeArena::new());
    let source = "export {}; interface Factory { new(): typeof holder.value } declare const Build: Factory; class Holder { value = new Build() } declare const holder: Holder;";
    let files = parse(&arena, &[("main.ts", source)]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&files[0]])),
        &HashSet::new(),
    );
    let lookup = tree.program_lookup("main.ts").unwrap();
    assert!(lookup
        .field_type_id_of(owner(&ids, &files[0], "value"))
        .is_none_or(|ty| matches!(arena.get(ty), Type::Unknown)));
}

#[test]
fn shared_initializer_source_has_distinct_results_in_overlapping_programs() {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("shared.ts", "export class Holder { value = new Build() }"),
        ("left.d.ts", "interface Left { left(): void } interface Factory { new(): Left } declare const Build: Factory;"),
        ("right.d.ts", "interface Right { right(): void } interface Factory { new(): Right } declare const Build: Factory;"),
        ("a.ts", "export {};"), ("b.ts", "export {};")]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut left = context(&[&files[0], &files[1], &files[3]])
        .programs
        .unwrap()
        .remove(0);
    let mut right = context(&[&files[0], &files[2], &files[4]])
        .programs
        .unwrap()
        .remove(0);
    left.key = "left".into();
    left.fingerprint = "left".into();
    right.key = "right".into();
    right.fingerprint = "right".into();
    let selected = crate::indexer::resolve::ProjectContext {
        programs: Some(vec![left, right]),
        ..Default::default()
    };
    let field = owner(&ids, &files[0], "value");
    let left = owner(&ids, &files[1], "Left");
    let right = owner(&ids, &files[2], "Right");
    let check = |tree: &Compilation| {
        let a = tree.program_lookup("a.ts").unwrap();
        let b = tree.program_lookup("b.ts").unwrap();
        let arena = tree.type_arena().unwrap();
        let ta = a.field_type_id_of(field).unwrap();
        let tb = b.field_type_id_of(field).unwrap();
        assert_eq!(head_decl_id(arena, ta), Some(left));
        assert_eq!(head_decl_id(arena, tb), Some(right));
        assert!(!(&a as &dyn SymbolLookup).accepts_type_context(arena, tb));
        assert!(!(&b as &dyn SymbolLookup).accepts_type_context(arena, ta));
        assert!(
            tree.program_lookup("shared.ts")
                .unwrap()
                .field_type_id_of(field)
                .is_none(),
            "ambiguous program selection cannot pick a view"
        );
    };
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&selected),
        &HashSet::new(),
    );
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    restored.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    check(&cold);
}
