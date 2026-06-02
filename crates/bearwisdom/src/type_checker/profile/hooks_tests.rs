use super::*;
use crate::type_checker::core::types::TypeArena;
use crate::types::{CallArg, EdgeKind, ExtractedRef};

fn make_ref() -> ExtractedRef {
    ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: 0,
        target_name: "x".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::<CallArg>::new(),
    }
}

#[test]
fn noop_hooks_preprocess_ref_leaves_ref_unchanged() {
    let hooks = NoOpHooks;
    let mut r = make_ref();
    let before = r.target_name.clone();
    hooks.preprocess_ref(&mut r);
    assert_eq!(r.target_name, before);
}

#[test]
fn noop_hooks_dispatch_returns_none() {
    let hooks = NoOpHooks;
    let r = make_ref();
    let ctx = DispatchContext {
        call_ref: &r,
        arg_types: &[],
        expected_return: None,
    };
    assert!(hooks.resolve_dispatch_special(&[1, 2, 3], &ctx).is_none());
}

#[test]
fn noop_hooks_detect_flow_emission_returns_none() {
    let hooks = NoOpHooks;
    let r = make_ref();
    let ctx = RefContext {
        file_path: "src/main.rs",
        language: "rust",
        owner_qname: None,
    };
    assert!(hooks.detect_flow_emission_special(&r, &ctx).is_none());
}

#[test]
fn enrich_external_type_is_a_pure_noop_on_default() {
    let hooks = NoOpHooks;
    let mut arena = TypeArena::new();
    let id = arena.class("ExternalThing");
    let pre_len = arena.len();
    hooks.enrich_external_type(id, &mut arena);
    assert_eq!(arena.len(), pre_len);
}
