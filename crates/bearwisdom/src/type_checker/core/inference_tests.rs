// =============================================================================
// type_checker/core/inference_tests.rs — Unit tests for inference.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::Resolution;
use crate::type_checker::core::types::{LitValue, PrimKind, Type, TypeArena};
use crate::type_checker::profile::language_profile::{DEFAULT_PROFILE, LanguageProfile};
use crate::types::{CallArg, EdgeKind, ExtractedRef};

fn bare_ref(kind: EdgeKind, target: &str) -> ExtractedRef {
    ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

fn ref_with_args(kind: EdgeKind, target: &str, args: Vec<CallArg>) -> ExtractedRef {
    let mut r = bare_ref(kind, target);
    r.call_args = args;
    r
}

fn resolution_with_yield_name(name: &str, arena: &TypeArena) -> Resolution {
    Resolution {
        target_symbol_id: 1,
        confidence: 1.0,
        strategy: "test",
        resolved_yield_type: Some(arena.class(name)),
        flow_emit: None,
    }
}

fn resolution_without_yield() -> Resolution {
    Resolution {
        target_symbol_id: 1,
        confidence: 1.0,
        strategy: "test",
        resolved_yield_type: None,
        flow_emit: None,
    }
}

#[test]
fn infer_returns_resolution_yield_type_when_present() {
    let mut arena = TypeArena::new();
    let res = resolution_with_yield_name("User", &arena);
    let r = bare_ref(EdgeKind::Calls, "doStuff");

    let out = infer_expression_type(&r, Some(&res), &mut arena, &DEFAULT_PROFILE)
        .expect("yield passes through");
    let user = arena.class("User");
    assert_eq!(out, user);
}

#[test]
fn infer_falls_through_when_resolution_has_no_yield() {
    let mut arena = TypeArena::new();
    let res = resolution_without_yield();
    let r = bare_ref(EdgeKind::Instantiates, "User");
    // Falls through to Instantiates handler → arena.class("User").
    let out = infer_expression_type(&r, Some(&res), &mut arena, &DEFAULT_PROFILE)
        .expect("Instantiates fallback");
    assert_eq!(out, arena.class("User"));
}

#[test]
fn infer_falls_back_to_class_for_instantiates() {
    let mut arena = TypeArena::new();
    let r = bare_ref(EdgeKind::Instantiates, "User");

    let out = infer_expression_type(&r, None, &mut arena, &DEFAULT_PROFILE);
    let user = arena.class("User");
    assert_eq!(out, Some(user));
}

#[test]
fn infer_returns_none_without_resolution_for_calls() {
    let mut arena = TypeArena::new();
    let r = bare_ref(EdgeKind::Calls, "doStuff");
    let out = infer_expression_type(&r, None, &mut arena, &DEFAULT_PROFILE);
    assert!(out.is_none());
}

#[test]
fn infer_literal_when_profile_enables_narrowing() {
    let mut arena = TypeArena::new();
    let profile = LanguageProfile {
        literal_narrowing: true,
        ..DEFAULT_PROFILE
    };
    let r = ref_with_args(
        EdgeKind::Calls,
        "foo",
        vec![CallArg::StringLit("hello".into())],
    );
    let out = infer_expression_type(&r, None, &mut arena, &profile).expect("literal inferred");
    assert_eq!(arena.get(out), Type::Literal(LitValue::Str("hello".into())));
}

#[test]
fn infer_parses_numeric_literal_when_narrowing_enabled() {
    let mut arena = TypeArena::new();
    let profile = LanguageProfile {
        literal_narrowing: true,
        ..DEFAULT_PROFILE
    };
    let r = ref_with_args(
        EdgeKind::Calls,
        "foo",
        vec![CallArg::Literal("42".into())],
    );
    let out = infer_expression_type(&r, None, &mut arena, &profile).expect("numeric inferred");
    assert_eq!(arena.get(out), Type::Literal(LitValue::Int(42)));
}

#[test]
fn infer_parses_bool_literal_when_narrowing_enabled() {
    let mut arena = TypeArena::new();
    let profile = LanguageProfile {
        literal_narrowing: true,
        ..DEFAULT_PROFILE
    };
    let r = ref_with_args(
        EdgeKind::Calls,
        "foo",
        vec![CallArg::Literal("true".into())],
    );
    let out = infer_expression_type(&r, None, &mut arena, &profile).expect("bool inferred");
    assert_eq!(arena.get(out), Type::Literal(LitValue::Bool(true)));
}

#[test]
fn infer_ignores_literal_call_args_without_narrowing_axis() {
    let mut arena = TypeArena::new();
    // DEFAULT_PROFILE has literal_narrowing = false.
    let r = ref_with_args(
        EdgeKind::Calls,
        "foo",
        vec![CallArg::StringLit("hello".into())],
    );
    let out = infer_expression_type(&r, None, &mut arena, &DEFAULT_PROFILE);
    assert!(out.is_none());
}

#[test]
fn unwrap_await_peels_async_wrapper() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let promise_user = arena.intern(Type::AsyncWrapper(user));
    assert_eq!(unwrap_await(promise_user, &arena), user);
}

#[test]
fn unwrap_await_is_identity_on_non_async() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    assert_eq!(unwrap_await(user, &arena), user);
}

#[test]
fn unwrap_iterator_peels_iterator_wrapper_when_axis_set() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let iter_user = arena.intern(Type::Iterator(user));

    let profile = LanguageProfile {
        single_inner_wrappers: &[],
        iterator_method: Some("next"),
        ..DEFAULT_PROFILE
    };
    assert_eq!(unwrap_iterator(iter_user, &arena, &profile), user);
}

#[test]
fn unwrap_iterator_peels_generic_apply_first_arg() {
    // Vec<User> → element type User, when language opts in.
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let vec_ty = arena.class("Vec");
    let vec_user = arena.intern(Type::Apply {
        base: vec_ty,
        args: vec![user],
    });

    let profile = LanguageProfile {
        single_inner_wrappers: &[],
        iterator_method: Some("next"),
        ..DEFAULT_PROFILE
    };
    assert_eq!(unwrap_iterator(vec_user, &arena, &profile), user);
}

#[test]
fn unwrap_iterator_is_identity_when_axis_unset() {
    // DEFAULT_PROFILE.iterator_method = None — peeling disabled.
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let iter_user = arena.intern(Type::Iterator(user));
    assert_eq!(unwrap_iterator(iter_user, &arena, &DEFAULT_PROFILE), iter_user);
}

#[test]
fn unwrap_iterator_is_identity_on_non_iterable_class() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let profile = LanguageProfile {
        single_inner_wrappers: &[],
        iterator_method: Some("next"),
        ..DEFAULT_PROFILE
    };
    assert_eq!(unwrap_iterator(user, &arena, &profile), user);
}
