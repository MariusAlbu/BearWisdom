use super::*;

#[test]
fn unwrap_preserves_the_inner_type_id() {
    let arena = TypeArena::new();
    let inner = arena.intern(Type::Tuple(vec![]));
    let base = arena.intern(Type::Class("Promise".into()));
    let applied = arena.intern(Type::Apply {
        base,
        args: vec![inner],
    });
    assert_eq!(unwrap_async_yield_id(applied, &arena, &["Promise"]), inner);
    assert_eq!(unwrap_async_yield_id(applied, &arena, &[]), applied);
    assert_eq!(unwrap_async_yield_id(inner, &arena, &["Promise"]), inner);
}

#[test]
fn legacy_string_adapter_keeps_existing_behavior() {
    assert_eq!(
        unwrap_async_yield_str("Promise<Response>", &["Promise"]),
        Some("Response")
    );
    assert_eq!(unwrap_async_yield_str("Response", &["Promise"]), None);
    assert_eq!(unwrap_async_yield_str("Promise<Response>", &[]), None);
}
