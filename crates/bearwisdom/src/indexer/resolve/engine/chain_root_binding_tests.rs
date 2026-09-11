use super::*;
use crate::indexer::resolve::engine::testkit::{
    call_ref, file_ctx, ref_ctx, source_symbol, Lookup,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

#[test]
fn active_binding_type_precedes_the_extractors_flat_annotation() {
    let arena = TypeArena::new();
    let lookup = Lookup::new().with_local_type("value", "Alpha");
    let reference = call_ref("save");
    let source = source_symbol("caller");
    let context = ref_ctx(&reference, &source, vec![]);
    let segment = crate::types::ChainSegment {
        name: "value".into(),
        kind: SegmentKind::Identifier,
        node_kind: String::new(),
        declared_type: Some("Beta".into()),
        declared_type_id: None,
        type_args: vec![],
        type_arg_ids: vec![],
        optional_chaining: false,
        byte_offset: 0,
        is_call: false,
        call_args: vec![],
    };
    let receiver = resolve_root_impl(
        &context,
        &file_ctx(vec![], None),
        &lookup,
        &arena,
        &DEFAULT_PROFILE,
        &segment,
    )
    .unwrap();
    assert_eq!(receiver.ty, arena.class("Alpha"));
}

fn type_access(name: &str) -> crate::types::ChainSegment {
    crate::types::ChainSegment {
        name: name.into(),
        kind: SegmentKind::TypeAccess,
        node_kind: String::new(),
        declared_type: None,
        declared_type_id: None,
        type_args: vec![],
        type_arg_ids: vec![],
        optional_chaining: false,
        byte_offset: 0,
        is_call: false,
        call_args: vec![],
    }
}

fn renaming_import() -> crate::indexer::resolve::engine::contract::ImportEntry {
    crate::indexer::resolve::engine::contract::ImportEntry {
        imported_name: "TestResponseAssert".into(),
        module_path: Some("Illuminate\\\\Testing".into()),
        alias: Some("PHPUnit".into()),
        is_wildcard: false,
        binding_kind: Some(SegmentKind::TypeAccess),
    }
}

/// `use Illuminate\\Testing\\TestResponseAssert as PHPUnit; PHPUnit::withResponse()`
/// must root on the renamed class, never on an unrelated class that happens to
/// be spelled like the alias.
#[test]
fn aliased_import_root_binds_the_original_not_a_same_named_stranger() {
    use crate::indexer::resolve::engine::testkit::sym;
    let arena = TypeArena::new();
    let lookup = Lookup::new()
        .with(sym(
            1,
            "TestResponseAssert",
            "Illuminate.Testing.TestResponseAssert",
            "class",
            "src/Illuminate/Testing/TestResponseAssert.php",
        ))
        .with(sym(
            2,
            "PHPUnit",
            "PHPUnit.Event.Runtime.PHPUnit",
            "class",
            "ext:idx:/vendor/phpunit/phpunit/src/Event/Runtime/PHPUnit.php",
        ));
    let reference = call_ref("withResponse");
    let source = source_symbol("caller");
    let context = ref_ctx(&reference, &source, vec![]);
    let receiver = resolve_root_impl(
        &context,
        &file_ctx(vec![renaming_import()], Some("Illuminate.Testing")),
        &lookup,
        &arena,
        &crate::languages::php::PHP_PROFILE,
        &type_access("PHPUnit"),
    )
    .expect("renamed root binds");
    assert_eq!(
        head_qname(&arena, receiver.ty).as_deref(),
        Some("Illuminate.Testing.TestResponseAssert")
    );
}

#[test]
fn aliased_import_without_its_declaration_is_a_miss_not_a_hijack() {
    use crate::indexer::resolve::engine::testkit::sym;
    let arena = TypeArena::new();
    let lookup = Lookup::new().with(sym(
        2,
        "PHPUnit",
        "PHPUnit.Event.Runtime.PHPUnit",
        "class",
        "ext:idx:/vendor/phpunit/phpunit/src/Event/Runtime/PHPUnit.php",
    ));
    let reference = call_ref("withResponse");
    let source = source_symbol("caller");
    let context = ref_ctx(&reference, &source, vec![]);
    let outcome = resolve_root_impl(
        &context,
        &file_ctx(vec![renaming_import()], Some("Illuminate.Testing")),
        &lookup,
        &arena,
        &crate::languages::php::PHP_PROFILE,
        &type_access("PHPUnit"),
    );
    let bound = outcome
        .as_ref()
        .ok()
        .and_then(|receiver| head_qname(&arena, receiver.ty));
    assert_ne!(
        bound.as_deref(),
        Some("PHPUnit.Event.Runtime.PHPUnit"),
        "the alias spelling must not bind the stranger"
    );
}
