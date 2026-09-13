// =============================================================================
// engine/chain_callable_value — what CALLING a value produces
//
// A value is callable in two shapes: its type is an INLINE signature
// (`const make: (o) => Client`), or its type is NOMINAL and the declaration it
// names carries a call signature (`interface F { (x: T): R }`, and the object
// type alias `type F = { (x: T): R }`). The extractor surfaces the second shape
// as a member named `call` on the declaration, so both reduce to one rule: read
// the signature's return, substituting the receiver's type arguments the way
// every member hop does.
//
// Every root that binds a value — by name, through its import scope, or through
// a source-bound lexical reference — asks the same question here, so a callable
// declaration types its call result identically whichever root found it.
// =============================================================================

use super::*;

#[cfg(test)]
#[path = "chain_callable_value_tests.rs"]
mod tests;

/// The member name the extractor synthesises for a declaration's call
/// signature (`interface F { (x): R }`).
const CALL_SIGNATURE_MEMBER: &str = "call";

/// What calling a value produces: the yielded type, plus the call-signature
/// declaration the yield was read from. `signature_id` is `None` for an inline
/// signature, which has no declaration of its own.
pub(super) struct CallYield {
    pub(super) ty: TypeId,
    pub(super) signature_id: Option<i64>,
}

/// The result of calling a value whose type is `value_ty`. `None` when that
/// type carries no call signature — the value is not callable, or the
/// signature's own return was never captured.
pub(super) fn call_yield(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    value_ty: TypeId,
) -> Option<CallYield> {
    if let Some(ty) = inline_signature_return(arena, value_ty) {
        return Some(CallYield {
            ty,
            signature_id: None,
        });
    }
    // A nominal type reaches its signature through its declaration: expand the
    // alias chain to the declaration that owns members, then walk to `call`.
    // Expansion may land on an inline signature (`type Make = (o) => Client`),
    // so the inline arm is retried on the expanded head.
    let recv = expand_receiver(Receiver::untyped(value_ty), lookup, arena, None, None);
    if let Some(ty) = inline_signature_return(arena, recv.ty) {
        return Some(CallYield {
            ty,
            signature_id: None,
        });
    }
    let call = lookup_member_on(lookup, arena, recv, CALL_SIGNATURE_MEMBER, &|_kind| true)?;
    let ty = yield_through(lookup, arena, &call, true, recv.ty, recv.id)?;
    Some(CallYield {
        ty,
        signature_id: Some(call.id),
    })
}

/// Whether `member_name` names the synthesised call signature itself.
pub(super) fn is_call_signature(member_name: &str) -> bool {
    member_name == CALL_SIGNATURE_MEMBER
}

/// The return of a value typed by an INLINE signature. `None` for a nominal
/// type, which reaches its signature through its declaration instead.
fn inline_signature_return(arena: &TypeArena, value_ty: TypeId) -> Option<TypeId> {
    match arena.get(value_ty) {
        Type::Function { return_, .. } => Some(return_),
        _ => None,
    }
}
