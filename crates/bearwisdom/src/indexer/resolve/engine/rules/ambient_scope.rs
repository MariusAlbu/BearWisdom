// =============================================================================
// engine/rules/ambient_scope — bare name resolves to a materialized
// ambient-scope symbol
//
// A package contributes symbols to *ambient scope* — globals referenceable
// without an import: a `lib.*.d.ts` top-level declaration, a `declare global`
// block, an `@types` global; a language's prelude / builtins / universe. The
// ecosystem layer flags those symbols at materialization; this rung binds a
// bare reference to one of them through the generic `ambient_symbols` surface.
//
// Language-agnostic: it fires for any ref whose bare target is in ambient
// scope, so one rung serves every language whose ecosystem populated it. Runs
// late in the ladder so a project symbol of the same name always wins.
//
// `resolve_ambient_named` is the shared core: the namespace-alias-strip and
// wildcard-fold rungs call it with a derived leaf name so all ambient binds
// share one kind-compatibility rule.
// =============================================================================

use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};
use crate::types::EdgeKind;

pub struct AmbientScopeRule;

impl LookupRule for AmbientScopeRule {
    fn name(&self) -> &'static str {
        "ambient_scope"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        // Bare names only — a dotted target is a member chain for an earlier rung.
        if target.is_empty() || target.contains('.') {
            return LookupResult::Pass;
        }
        resolve_ambient_named(ctx, target, "ambient_scope")
    }
}

/// Bind `name` (a bare leaf) to a kind-compatible ambient-scope symbol, tagged
/// with `strategy`, or `Pass`. Exposed so the namespace-alias-strip and
/// wildcard-fold rungs resolve a derived leaf through the same rule.
///
/// Relaxes the kind check for `Instantiates` against a `variable` — core-lib
/// constructors are declared as `declare var X: { new(): Y }`.
pub(super) fn resolve_ambient_named(
    ctx: &BinderContext<'_>,
    name: &str,
    strategy: &'static str,
) -> LookupResult {
    let edge_kind = ctx.edge_kind();
    for sym in ctx.lookup.ambient_symbols(name) {
        let kind_ok = (ctx.kind)(edge_kind, &sym.kind)
            || (edge_kind == EdgeKind::Instantiates && sym.kind == "variable");
        if kind_ok {
            return LookupResult::Resolved(ctx.resolved(sym.id, strategy));
        }
    }
    LookupResult::Pass
}

#[cfg(test)]
#[path = "ambient_scope_tests.rs"]
mod tests;
