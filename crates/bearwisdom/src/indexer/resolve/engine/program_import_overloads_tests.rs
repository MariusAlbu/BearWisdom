// =============================================================================
// program_import_overloads_tests — an imported overload group binds by arguments
// =============================================================================

use super::super::super::merge_proof::tests::{context, parse};
use crate::indexer::resolve::engine::{
    compilation::Compilation,
    contract::{FlowCacheLookup, SymbolLookup},
    file_lookup::FileLookup,
    semantic_model::{SemanticModel, SolveOutcome},
    testkit,
};
use crate::type_checker::core::types::{PrimKind, TypeArena};
use std::{collections::HashSet, sync::Arc};

const LIB: &str = "export declare function deco(): number;\nexport declare function deco(name: string): string;\nexport declare function single(): void;\n";
const MAIN: &str = "import { deco, single } from './lib';\nexport const a = deco();\nexport const b = deco('x');\nexport const c = single();\n";

struct Fixture {
    arena: Arc<TypeArena>,
    files: Vec<crate::types::ParsedFile>,
    ids: crate::indexer::symbol_ids::SymbolIds,
    tree: Compilation,
}

fn fixture() -> Fixture {
    let arena = Arc::new(TypeArena::new());
    let files = parse(&arena, &[("lib.ts", LIB), ("main.ts", MAIN)]);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &files,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let config = context(&[&files[0], &files[1]]);
    let tree = Compilation::build_with_context(
        &files,
        &ids,
        Arc::clone(&arena),
        Some(&config),
        &HashSet::new(),
    );
    Fixture {
        arena,
        files,
        ids,
        tree,
    }
}

#[test]
fn arguments_select_a_signature_in_declaration_order_and_name_its_row() {
    let fx = fixture();
    let lookup = FileLookup::for_file(&fx.tree, &fx.files[1], &fx.ids);
    let zero_args = MAIN.find("deco()").unwrap() as u32;
    let string_arg = MAIN.find("deco('x')").unwrap() as u32;

    let first = lookup
        .overloaded_import_call(zero_args, &[], &[])
        .expect("an imported overload group is attested")
        .expect("the zero-parameter signature accepts a zero-argument call");
    // Argument types come from the source's own operands, as the binder types them.
    let args = lookup.source_call_arguments(string_arg).unwrap().unwrap();
    let actual =
        crate::indexer::resolve::engine::arg_types::resolve_arg_types(&lookup, &fx.arena, args);
    let second = lookup
        .overloaded_import_call(string_arg, &actual, &[])
        .unwrap()
        .expect("the string-parameter signature accepts a string argument");

    let picked = |call: &crate::indexer::resolve::engine::contract::flow_cache::OverloadCall| {
        call.origins[call.selected].clone()
    };
    let (a, b) = (picked(&first), picked(&second));
    assert!(
        a.span.start < b.span.start,
        "the zero-argument call selects the earlier declaration"
    );
    assert_ne!(a.declaration, b.declaration, "each call binds its own row");
    for origin in [&a, &b] {
        let row = fx.tree.symbol_by_id(origin.declaration.unwrap()).unwrap();
        assert_eq!((row.name.as_str(), &*row.file_path), ("deco", "lib.ts"));
    }
    assert_eq!(
        fx.arena.format_type(first.return_type),
        "number",
        "the selected signature's result is the call's yield"
    );
}

#[test]
fn a_call_no_signature_accepts_is_an_authoritative_miss() {
    let fx = fixture();
    let lookup = FileLookup::for_file(&fx.tree, &fx.files[1], &fx.ids);
    let site = MAIN.find("deco('x')").unwrap() as u32;
    let number = fx.arena.primitive(PrimKind::Int);
    assert!(matches!(
        lookup.overloaded_import_call(site, &[number, number], &[]),
        Some(Err(()))
    ));
}

#[test]
fn a_single_declaration_import_is_not_an_overload_site() {
    let fx = fixture();
    let lookup = FileLookup::for_file(&fx.tree, &fx.files[1], &fx.ids);
    let site = MAIN.find("single()").unwrap() as u32;
    assert!(lookup.overloaded_import_call(site, &[], &[]).is_none());
}

#[test]
fn the_semantic_model_binds_a_bare_overloaded_call_to_the_selected_row() {
    let fx = fixture();
    let lookup = FileLookup::for_file(&fx.tree, &fx.files[1], &fx.ids);
    let site = MAIN.find("deco('x')").unwrap() as u32;
    let mut reference = testkit::call_ref("deco");
    reference.byte_offset = site;
    reference.call_args = vec![crate::types::CallArg::StringLit("x".into())];
    let source = testkit::source_symbol("b");
    let context = testkit::ref_ctx(&reference, &source, vec![]);
    let file = testkit::file_ctx(vec![], None);
    lookup.set_cursor(site);
    let profile = &crate::languages::typescript::profile::TYPESCRIPT_PROFILE;
    let SolveOutcome::Resolved(info) =
        SemanticModel::production().get_symbol_info(&context, &file, &lookup, profile)
    else {
        panic!("an attested overload group binds the call");
    };
    assert_eq!(info.strategy, "lexical_overload");
    let row = fx.tree.symbol_by_id(info.target_symbol_id).unwrap();
    assert_eq!(row.name, "deco");
    assert_eq!(
        row.signature.as_deref().map(|s| s.contains("name")),
        Some(true),
        "the string call selects the `name: string` signature"
    );
}
