// =============================================================================
// engine/rules/generic_param — generic-parameter-in-scope resolution
//
// A bare identifier whose name matches a generic parameter declared on the
// source symbol or any enclosing scope (impl/class/function) resolves to the
// declaring symbol. This counts the ref as an edge into that scope rather than
// polluting `unresolved_refs`. Catches `I`, `F`, `TDocSet`, `TSortKey`,
// `TScoreCombiner`, etc. across every language with declared-generic syntax.
//
// SymbolInfo order:
//   1. Arena-interned generic params on the source symbol (when the index
//      carries a TypeArena — skipped by test lookups that return `None`).
//   2. String-typed `generic_params` on the source symbol's qname (covers
//      languages whose params appear only in the symbol's own signature).
//   3. Enclosing scope_chain entries (impl block, outer class, etc.).
//
// A dotted or qualified target is not a generic parameter — declined early.
// =============================================================================

use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct GenericParamRule;

impl LookupRule for GenericParamRule {
    fn name(&self) -> &'static str {
        "generic_param"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        if target.is_empty() || target.contains('.') || target.contains("::") {
            return LookupResult::Pass;
        }

        // Arena path: source symbol's generic params interned into the workspace
        // TypeArena. Only fires when the index supplies an arena (i.e. not in
        // synthetic test lookups).
        if let Some(arena) = ctx.lookup.type_arena() {
            if ctx
                .ref_ctx
                .source_symbol
                .generic_params
                .iter()
                .any(|id| arena.generic_param(*id).name == target)
            {
                if let Some(sym) = ctx
                    .lookup
                    .by_qualified_name(&ctx.ref_ctx.source_symbol.qualified_name)
                {
                    return LookupResult::Resolved(ctx.resolved(sym.id, "engine_generic_param"));
                }
            }
        }

        // Source symbol's OWN declared params via the string map (populated from
        // signatures at index build). Covers the case where a template function's
        // params appear only in its own signature, never in an enclosing scope.
        // The scope-chain loop below deliberately skips the source symbol's qname,
        // so without this branch those self-declared params stay unresolved.
        if let Some(params) = ctx
            .lookup
            .generic_params(&ctx.ref_ctx.source_symbol.qualified_name)
        {
            if params.iter().any(|p| p == target) {
                if let Some(sym) = ctx
                    .lookup
                    .by_qualified_name(&ctx.ref_ctx.source_symbol.qualified_name)
                {
                    return LookupResult::Resolved(ctx.resolved(sym.id, "engine_generic_param"));
                }
            }
        }

        // Enclosing scopes: impl block around a method, class around a method on
        // a generic class, etc. scope_chain holds qnames; the first match wins.
        for scope_qname in ctx.ref_ctx.scope_chain.iter() {
            if scope_qname == &ctx.ref_ctx.source_symbol.qualified_name {
                continue;
            }
            let Some(params) = ctx.lookup.generic_params(scope_qname) else {
                continue;
            };
            if params.iter().any(|p| p == target) {
                if let Some(sym) = ctx.lookup.by_qualified_name(scope_qname) {
                    return LookupResult::Resolved(ctx.resolved(sym.id, "engine_generic_param"));
                }
            }
        }

        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "generic_param_tests.rs"]
mod tests;
