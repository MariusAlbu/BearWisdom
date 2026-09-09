//! Root binding and lexical-shadow barriers, separated from member walking.
use super::*;

#[cfg(test)]
#[path = "chain_root_binding_tests.rs"]
mod tests;

/// Inner body of `resolve_root`. See `resolve_root` for the contract.
pub(super) fn resolve_root_impl(
    ref_ctx: &RefContext,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    seg: &crate::types::ChainSegment,
) -> Result<Receiver, Option<Cause>> {
    if seg.kind == SegmentKind::BaseRef {
        return super::super::base_receiver::root(ref_ctx, lookup, arena).ok_or(None);
    }
    if let Some(local) = lookup.local_reference(ref_ctx.extracted_ref.byte_offset) {
        return super::lexical_root::resolve(local, lookup, arena, file_ctx, seg);
    }
    // Prefer the TypeId cache: `local_type_id` returns the TypeId that was
    // stored directly by `record_local_type_id`, preserving the exact type
    // variant (Primitive, Optional, Generic) without a format/intern round-trip.
    // Fall back to the String cache and intern once at this boundary when no
    // TypeId binding exists (e.g. bindings recorded via the String path or by
    // synthetic test doubles that only implement `local_type`).
    // A binding typed `ReturnType<typeof fn>` (or any return-type-extraction alias
    // of that shape) roots on `fn`'s return type, resolved in this file's import
    // scope so the right overload is chosen. Applies to both the TypeId-cached and
    // String-cached local bindings.
    if let Some(id) = lookup.local_type_id(&seg.name) {
        if let Some(r) = resolve_return_type_extraction(id, lookup, arena, file_ctx) {
            return Ok(Receiver::untyped(r));
        }
        return Ok(Receiver::untyped(id));
    }
    if let Some(ty) = lookup.local_type(&seg.name) {
        let id = arena.intern_type_str(&ty);
        if let Some(r) = resolve_return_type_extraction(id, lookup, arena, file_ctx) {
            return Ok(Receiver::untyped(r));
        }
        return Ok(Receiver::untyped(id));
    }
    if lookup.has_local_binding(&seg.name) {
        return Err(lookup
            .root_cause_hint(&seg.name)
            .or(Some(Cause::new(None, CauseKind::UntypedBinding))));
    }
    if let Some(ty) = &seg.declared_type {
        return Ok(Receiver::untyped(with_segment_args(
            arena,
            arena.intern_type_str(ty),
            &seg.type_args,
        )));
    }
    if matches!(seg.kind, SegmentKind::SelfRef) {
        // `this`/`self` roots on the enclosing type — the one declaration whose
        // members the chain walks. Bind its id so an inherited-member climb keys
        // on identity, not the enclosing type's qname string. The id-keyed
        // enclosing lookup runs first: the qname map answers by source qname,
        // which a same-named declaration in another package shares.
        if let Some(enc) = ref_ctx
            .source_symbol_id
            .and_then(|sid| lookup.enclosing_type_id_of(sid))
            .and_then(|eid| lookup.symbol_by_id(eid))
        {
            let ty = super::super::head_decl::nominal_head(lookup, arena, &enc);
            return Ok(Receiver {
                ty,
                id: Some(enc.id),
            });
        }
        if let Some(enc_qname) = lookup.enclosing_type_qname(&ref_ctx.source_symbol.qualified_name)
        {
            let ty = arena.class(enc_qname);
            let id = lookup.by_qualified_name(enc_qname).map(|s| s.id);
            return Ok(Receiver { ty, id });
        }
        // `enclosing_type_qname` walks `parent_index`, which is empty for a
        // language whose methods are extracted as AST siblings of their
        // container rather than nested children (Rust impl blocks). Fall back
        // to the scope chain (innermost first), then the source symbol's own
        // `scope_path`, accepting the first type-kind symbol either names.
        let enc_sym = ref_ctx
            .scope_chain
            .iter()
            .find_map(|q| {
                lookup
                    .by_qualified_name(q)
                    .filter(|s| is_type_kind(&s.kind))
            })
            .or_else(|| {
                let sp = ref_ctx.source_symbol.scope_path.as_deref()?;
                lookup
                    .by_qualified_name(sp)
                    .filter(|s| is_type_kind(&s.kind))
            })
            .ok_or(None)?;
        let ty = super::super::head_decl::nominal_head(lookup, arena, enc_sym);
        return Ok(Receiver {
            ty,
            id: Some(enc_sym.id),
        });
    }
    // An import-bound root types through its import's own candidate set —
    // external ext-files, the internally-linked module file, or the named
    // workspace package — or dies with the import as its cause. The unscoped
    // by-name fallbacks below never run for such a root: a same-named symbol
    // from an unrelated file is a hijack, not a resolution.
    match super::super::root_import_discipline::apply(file_ctx, lookup, arena, seg) {
        RootImportOutcome::Typed(recv) => return Ok(recv),
        RootImportOutcome::Deny(c) => return Err(Some(c)),
        RootImportOutcome::Unconstrained => {}
    }

    // Nothing typed the root outright. The remaining strategies may still find
    // the SYMBOL the root names — just discover its own type was never
    // captured. Keep the most specific such near-miss; it beats a blank
    // unbound_root when every strategy below also comes up empty.
    let mut cause: Option<Cause> = None;

    if seg.is_call {
        match resolve_callee_return_and_id(lookup, arena, file_ctx, &seg.name, None) {
            Ok((ty, id)) => {
                // A root call's explicit type arguments bind the callee's own
                // generic params before argument inference fills the rest.
                let ty = lookup
                    .by_name(&seg.name)
                    .iter()
                    .find(|s| s.id == id)
                    .map(|callee| bind_explicit_type_args(lookup, arena, callee, seg, ty))
                    .unwrap_or(ty);
                let ty = bind_call_args_into_return(
                    lookup,
                    arena,
                    id,
                    seg.byte_offset,
                    &seg.call_args,
                    ty,
                );
                return Ok(Receiver::untyped(ty));
            }
            Err(c) => cause = cause.or(c),
        }
        // The callee is not a callable declaration (function/method) but may be
        // a VALUE whose declared type is a callable interface — `const v: I`
        // where `I` carries a call signature. Calling it yields the call
        // signature's return, not the interface type itself, so this must precede
        // the value-root fallthrough below.
        if let Some(ty) = call_value_root_type(
            lookup,
            arena,
            &seg.name,
            &ref_ctx.source_symbol.qualified_name,
            file_ctx,
            ref_ctx.file_package_id,
        ) {
            return Ok(Receiver::untyped(ty));
        }
    }
    // Import-of-value / typed-value root: a value (a `declare const`, an
    // imported binding, a typed field) whose declaration carries a type roots
    // the chain on that type — `builder.create()` roots on `builder`'s declared
    // type even though `builder` is not itself a type name.
    match value_root_type(
        lookup,
        arena,
        &seg.name,
        &ref_ctx.source_symbol.qualified_name,
        file_ctx,
        ref_ctx.file_package_id,
    ) {
        Ok(ty) => {
            // A value whose declared type is `ReturnType<typeof f>` — an inferred
            // `const x = f(...)` binding imported from the file that declares it —
            // roots on f's return type (f resolved globally by name), deriving the
            // cross-file type the per-file flow seed could not carry.
            if let Some(r) = resolve_return_type_extraction(ty, lookup, arena, file_ctx) {
                return Ok(Receiver::untyped(r));
            }
            return Ok(Receiver::untyped(ty));
        }
        Err(c) => cause = cause.or(c),
    }
    // Bare type name used as a static-access / construction root. When the same
    // name is declared in several sibling workspace packages, prefer the
    // declaration in the package the use site imports the name from; otherwise
    // fall back to the first same-named type. The resolved `Symbol` IS the
    // receiver's declaration, so bind its id directly rather than round-tripping
    // its qname back through `by_qualified_name`.
    let candidates = lookup.types_by_name(&seg.name);
    let cand_refs: Vec<&Symbol> = candidates.iter().collect();
    // Prefer the import-scoped declaration — the package/module the use site
    // imports `name` from — over a first-winner same-name pick (a `Page` from the
    // package the file imports, not a same-named `Page` in another). Ranked +
    // ascending-id deterministic; when no candidate clearly wins, the first
    // same-named type is the fallback.
    let Some(s) = pick_ranked_candidate(file_ctx, ref_ctx.file_package_id, lookup, &cand_refs)
        .or_else(|| candidates.first())
    else {
        // A prior forward-inference pass may have already diagnosed why THIS
        // EXACT binding carries no seeded type — that names the true upstream
        // cause (an initializer's uncaptured return/field), which outranks the
        // generic "this binding itself is untyped" signal collected above.
        return Err(lookup.root_cause_hint(&seg.name).or(cause));
    };
    let ty = with_segment_args(
        arena,
        super::super::head_decl::nominal_head(lookup, arena, s),
        &seg.type_args,
    );
    Ok(Receiver::new(ty, s.id))
}
