// =============================================================================
// engine/chain_callee_root.rs — the callable a bare call names, and its yield
//
// A bare call `f(...)` with no receiver picks its callee from the same-named
// candidates: a callee nested in the enclosing scope first, then the
// import-scoped overload set, then the first free function or constructor
// declaration. The yield is the callee's recorded return (object-literal
// return, id-keyed slot, qname slot, then the stored return text); a callee
// with no recorded return is reported as the diagnosable near-miss.
// =============================================================================

use super::*;

/// The return type the callable `name` resolves to in this file's import scope,
/// paired with the SYMBOL ID of the declaration it was read from. The id lets a
/// caller read that declaration's generic params (to bind a call's type
/// arguments). See `callee_return_type` for the resolution order — this is its
/// id-carrying form.
pub(super) fn resolve_callee_return_and_id(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file_ctx: &FileContext,
    name: &str,
    enclosing_scope: Option<&str>,
) -> Result<(TypeId, i64), Option<Cause>> {
    let candidates = lookup.by_name(name);
    // The most specific callable candidate found so far whose own return type
    // was never captured — the diagnosable near-miss if every strategy below
    // falls through without a typed return.
    let mut untyped_callee: Option<i64> = None;

    // Scope preference: a callee declared INSIDE `enclosing` — a nested
    // `function inner(){…}` whose qname is `{enclosing}.{name}` — is the one a
    // factory's `return inner()` means, resolved before the unscoped name
    // fallback picks an arbitrary namesake. Its `$Ret` (object-literal return)
    // is authoritative, same as the general path below.
    if let Some(scoped_qname) = enclosing_scope.map(|e| join_index_qname(e, name)) {
        let scoped_id = lookup.by_qualified_name(&scoped_qname).map(|s| s.id);
        if let Some(cand) = candidates
            .iter()
            .find(|s| is_callable(&s.kind) && Some(s.id) == scoped_id)
        {
            let ret_qname = index_return_qname(&cand.qualified_name);
            if lookup.by_qualified_name(&ret_qname).is_some() {
                return Ok((arena.class(&ret_qname), cand.id));
            }
            if let Some(id) = super::super::type_slots::return_type_by_identity(lookup, cand) {
                return Ok((id, cand.id));
            }
            if let Some(id) = lookup
                .return_type_str(&cand.qualified_name)
                .and_then(|text| intern_lookup_type_text(lookup, arena, &text))
            {
                return Ok((id, cand.id));
            }
            // Scoped callee exists but carries no recorded return — fall through.
            untyped_callee = Some(cand.id);
        }
    }
    // An object-literal return synthesized as `{qname}$Ret` (the flow-return-object
    // pass) IS the function's structural return — authoritative over any stored
    // return inferred from a param annotation (`Record`) or a body expression
    // (`Promise`). Prefer it before reading the stored slot.
    for cand in candidates.iter().filter(|s| is_callable(&s.kind)) {
        let ret_qname = index_return_qname(&cand.qualified_name);
        if lookup.by_qualified_name(&ret_qname).is_some() {
            return Ok((arena.class(&ret_qname), cand.id));
        }
    }
    // Import-scoped overload set: when `name` is imported from a specific
    // package, the chain heads on THAT package's declaration. An OVERLOADED
    // function records its return on ONE specific signature (often the
    // implementation, not the first overload), so scan the scoped callables for
    // the one carrying a per-id return rather than blindly taking the first —
    // the root-typing parity the call-ref path gets by binding the arg-matched
    // overload. Prefer the id-keyed return over the qname slot, which a
    // same-named declaration in another package may have won.
    if let Some(pkg) = import_scoped_package_id(file_ctx, lookup, name) {
        let scoped: Vec<_> = candidates
            .iter()
            .filter(|s| is_callable(&s.kind) && s.package_id == Some(pkg))
            .collect();
        for s in &scoped {
            if let Some(id) = lookup.return_type_id_of(s.id) {
                return Ok((id, s.id));
            }
        }
        if let Some(callee) = scoped.first() {
            if let Some(id) = super::super::type_slots::return_type_by_identity(lookup, callee) {
                return Ok((id, callee.id));
            }
            if let Some(id) = lookup
                .return_type_str(&callee.qualified_name)
                .and_then(|text| intern_lookup_type_text(lookup, arena, &text))
            {
                return Ok((id, callee.id));
            }
            untyped_callee.get_or_insert(callee.id);
        }
    }
    // No import attribution (or the scoped set yielded no return): the first
    // free-function or constructor declaration of this name — a language that
    // spells construction as a bare application binds the constructor row, whose
    // recorded return is the declaring type. Methods require an explicit
    // receiver and must not root a bare unscoped call — they are excluded here
    // so an unrelated method named the same as an ambient callable-interface
    // const does not shadow the const's call-signature path.
    let Some(callee) = candidates
        .iter()
        .find(|s| s.kind == "function" || super::super::kinds::is_constructor_kind(&s.kind))
    else {
        // Not a callable declaration — the name may bind a VALUE whose own type
        // carries the call signature (`declare const make: (opts) => Client<…>`,
        // `declare const check: CheckStatic`); calling it yields that
        // signature's return.
        for cand in candidates.iter().filter(|s| is_value_kind(&s.kind)) {
            let Some(vty) = field_type_of(lookup, arena, cand.id, &cand.qualified_name) else {
                continue;
            };
            if let Some(yielded) = super::callable_value::call_yield(lookup, arena, vty) {
                return Ok((yielded.ty, cand.id));
            }
        }
        return Err(untyped_callee.map(|id| Cause::new(Some(id), CauseKind::UncapturedReturn)));
    };
    if let Some(id) = lookup.return_type_id_of(callee.id) {
        return Ok((id, callee.id));
    }
    if let Some(id) = lookup.return_type_id(&callee.qualified_name) {
        return Ok((id, callee.id));
    }
    if let Some(id) = lookup
        .return_type_str(&callee.qualified_name)
        .and_then(|text| intern_lookup_type_text(lookup, arena, &text))
    {
        return Ok((id, callee.id));
    }
    Err(Some(Cause::new(
        Some(callee.id),
        CauseKind::UncapturedReturn,
    )))
}
