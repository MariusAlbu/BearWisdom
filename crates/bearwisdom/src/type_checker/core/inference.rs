// =============================================================================
// type_checker/core/inference.rs — RHS type inference + wrapper unwrappers
//
// The engine's side of "what type does this expression yield?". Three
// entry points:
//   - `infer_expression_type` — given a resolved ref (or no resolution at
//     all), produce the TypeId that should bind to the LHS receiver.
//   - `unwrap_await` — peel one `AsyncWrapper` so `await foo()` is typed
//     as the underlying return value.
//   - `unwrap_iterator` — peel one iterator wrapper so `for x in xs` binds
//     `x` to the element type rather than to `xs` itself.
//
// Spec: research/architecture/02-engine-internal-architecture.html § Layer 3
//       research/architecture/04-implementation-phases.html § Phase 3
// =============================================================================

use super::types::{Type, TypeArena, TypeId};
use crate::indexer::resolve::engine::Resolution;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{EdgeKind, ExtractedRef};

/// Infer the yielded TypeId of an expression reference.
///
/// The fast path is "the chain walker already resolved this ref to a
/// symbol, and the resolution carries `resolved_yield_type` — return it".
/// The fallback covers refs the resolver hasn't classified yet, currently
/// just the `Instantiates` shape (`new Foo()` / `Foo()` callable-class).
/// Literal-narrowing inference is gated on the profile axis: languages that
/// widen literals at runtime (most) leave the literal type behind, while
/// languages that preserve singleton types in surface types (TS strict
/// mode) carry them through.
pub fn infer_expression_type(
    expr_ref: &ExtractedRef,
    resolution: Option<&Resolution>,
    arena: &TypeArena,
    profile: &LanguageProfile,
) -> Option<TypeId> {
    // Resolution.resolved_yield_type is now TypeId-native — engine produces
    // the TypeId directly via the chain walker / bare-name resolver and the
    // consumer reads it without re-interning.
    if let Some(res) = resolution {
        if let Some(id) = res.resolved_yield_type {
            return Some(id);
        }
    }

    match expr_ref.kind {
        EdgeKind::Instantiates => Some(arena.class(&expr_ref.target_name)),
        _ if profile.literal_narrowing => infer_from_call_args(expr_ref, arena),
        _ => None,
    }
}

/// Walk one `await` shell off `ty`. Returns `ty` unchanged when it is not
/// wrapped in `AsyncWrapper`, so chain walkers that always call this can
/// stay branchless on the receive side.
pub fn unwrap_await(ty: TypeId, arena: &TypeArena) -> TypeId {
    match arena.get(ty) {
        Type::AsyncWrapper(inner) => inner,
        _ => ty,
    }
}

/// Walk one iterator shell off `ty`. Recognized shapes:
///   - `Type::Iterator(inner)` — engine-canonical wrapper. Always unwrapped.
///   - `Type::Apply { base, args }` where `args` is non-empty: treated as
///     a generic iterable. The first type argument is taken as the
///     element type. This covers `Vec<T>`, `List<T>`, `Set<T>`,
///     `Iterator<T>`, `IEnumerable<T>` uniformly.
///
/// The profile gate (`iterator_method`) is consulted for languages that
/// can't surface iterator types at extraction time — when absent, no
/// iteration peeling happens and `ty` returns unchanged. The chain walker
/// is responsible for falling back to receiver-method dispatch in that
/// case.
pub fn unwrap_iterator(ty: TypeId, arena: &TypeArena, profile: &LanguageProfile) -> TypeId {
    if profile.iterator_method.is_none() {
        // Conservative default — language hasn't opted in to engine-side
        // iteration peeling. Caller keeps the original receiver.
        return ty;
    }
    match arena.get(ty) {
        Type::Iterator(inner) => inner,
        Type::Apply { args, .. } if !args.is_empty() => args[0],
        _ => ty,
    }
}

/// Single-literal fallback used when `profile.literal_narrowing` is true and
/// no explicit resolution carries a yield type. Pulls the first positional
/// call argument and, when it is a recognisable literal shape, interns a
/// `Type::Literal` from it. Returns None otherwise — the chain walker
/// records a clean miss rather than committing to Unknown.
///
/// Recognised:
///   - `StringLit(s)` → `Literal(Str(s))`.
///   - `Literal(src)` parsing as `true` / `false` → `Literal(Bool(_))`.
///   - `Literal(src)` parsing as integer → `Literal(Int(_))`.
fn infer_from_call_args(expr_ref: &ExtractedRef, arena: &TypeArena) -> Option<TypeId> {
    use crate::type_checker::core::types::LitValue;
    use crate::types::CallArg;

    let arg = expr_ref.call_args.first()?;
    match arg {
        CallArg::StringLit(s) => Some(arena.intern(Type::Literal(LitValue::Str(s.clone())))),
        CallArg::Literal(src) => parse_simple_literal(src).map(|lv| arena.intern(Type::Literal(lv))),
        _ => None,
    }
}

fn parse_simple_literal(src: &str) -> Option<crate::type_checker::core::types::LitValue> {
    use crate::type_checker::core::types::LitValue;
    let trimmed = src.trim();
    match trimmed {
        "true" => Some(LitValue::Bool(true)),
        "false" => Some(LitValue::Bool(false)),
        _ => trimmed.parse::<i64>().ok().map(LitValue::Int),
    }
}

#[cfg(test)]
#[path = "inference_tests.rs"]
mod tests;
