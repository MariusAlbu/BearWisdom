use super::contract::chain_walker::parse_type_head_and_args;
use crate::type_checker::core::types::{Type, TypeArena, TypeId};

// ---------------------------------------------------------------------------
// Async-wrapper unwrap helper
// ---------------------------------------------------------------------------

/// Peel one async-wrapper layer from `yield_id` when the binding was `await`-ed.
///
/// Handles two forms:
///   - `Type::Apply { base: <wrapper-class>, args: [T] }` where the wrapper
///     class name is in `profile.async_wrappers` — returns `args[0]` (T).
///   - Bare wrapper head with no applied arg — returns `yield_id` unchanged.
///
/// The unwrap fires ONLY at the binding seed (the binding's await flag gates it);
/// a non-awaited `Promise<T>` variable is never touched.
pub(super) fn unwrap_async_yield_id(
    yield_id: TypeId,
    arena: &TypeArena,
    async_wrappers: &[&str],
) -> TypeId {
    if async_wrappers.is_empty() {
        return yield_id;
    }
    if let Type::Apply { base, args } = arena.get(yield_id) {
        if !args.is_empty() {
            if let Type::Class(head) = arena.get(base) {
                if async_wrappers.contains(&head.as_str()) {
                    return args[0];
                }
            }
        }
    }
    yield_id
}

/// Peel one async-wrapper layer from a type string (the String seed path).
///
/// `"Promise<Response>"` → `"Response"` when `"Promise"` is in `async_wrappers`.
/// Returns `None` when the string has no wrapper head or the first arg is empty,
/// so the caller keeps the original string unchanged.
pub(super) fn unwrap_async_yield_str<'a>(ty: &'a str, async_wrappers: &[&str]) -> Option<&'a str> {
    if async_wrappers.is_empty() {
        return None;
    }
    let (head, args) = parse_type_head_and_args(ty);
    if args.is_empty() {
        return None;
    }
    if async_wrappers.contains(&head) {
        let inner = args[0].trim();
        if !inner.is_empty() {
            return Some(inner);
        }
    }
    None
}

#[cfg(test)]
#[path = "async_yield_tests.rs"]
mod tests;
