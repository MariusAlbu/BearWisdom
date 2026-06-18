// =============================================================================
// engine/semantic_model — the new engine's entry: solve one code reference
//
// The `SemanticModel` owns the production rule ladder and resolves a ref against
// it. Mirrors the old engine's `resolve_generic` entry: build the profile-driven
// kind predicate, assemble a `BinderContext`, run the rules. Chain-bearing refs
// (member access) are walked by the reused `ChainWalker` structure — wired in a
// later step; this entry covers the chain-less ladder the lifted rules
// implement.
// =============================================================================

use std::str::FromStr;

use crate::indexer::resolve::engine::contract::{FileContext, RefContext, SymbolInfo, SymbolLookup};
use crate::type_checker::profile::language_profile::{
    KindCompatibility, KindTable, LanguageProfile,
};
use crate::types::{EdgeKind, SymbolKind};

use super::{BinderContext, Binder};

/// The rule-based code-reference solver. Holds the ordered rule set and applies
/// it to one ref at a time.
pub struct SemanticModel {
    engine: Binder,
}

impl SemanticModel {
    /// Build a solver over the production rule ladder.
    pub fn production() -> Self {
        Self {
            engine: Binder::production(),
        }
    }

    /// Build a solver over an explicit rule set (tests inject a subset).
    pub fn new(engine: Binder) -> Self {
        Self { engine }
    }

    /// Solve one ref. A chain-bearing ref (member access) walks the Symbol tree;
    /// a chain-less ref runs the rule ladder. There is no old-engine fallback —
    /// a ref no rule and no chain hop resolves is honestly unresolved.
    pub fn get_symbol_info(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
    ) -> Option<SymbolInfo> {
        if let Some(chain) = ref_ctx.extracted_ref.chain.as_ref() {
            if let Some(res) = super::chain::bind_member_access(ref_ctx, file_ctx, lookup) {
                return Some(res);
            }
            // A multi-segment chain the walk declined is normally a genuine miss:
            // a same-named sibling must not hijack `a.b.c`. The exception is
            // NAMESPACE-qualified member access — `React.useState` where `React`
            // names a namespace/module, not a value. The chain walker can't root
            // on a namespace, but the bare-name ladder resolves the member under
            // the imported module (`imported_namespace` / `ref_module`), and the
            // module scope keeps it from binding an unrelated sibling. A
            // single-segment "chain" carries no receiver and always falls through.
            if chain.segments.len() > 1 && !chain_root_is_namespace(chain, lookup) {
                return None;
            }
        }
        self.resolve_chain_less(ref_ctx, file_ctx, lookup, profile)
            .map(|(res, _rule)| res)
    }

    /// Solve one chain-less ref through the rule ladder. Builds the profile kind
    /// predicate, assembles a `BinderContext`, and runs the rules. Returns the
    /// resolution and the name of the rule that produced it, or `None` when the
    /// ladder declined — an honestly-unresolved ref.
    pub fn resolve_chain_less(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
    ) -> Option<(SymbolInfo, &'static str)> {
        let table = profile.kind_compatible_table;
        let kind = move |edge: EdgeKind, sym_kind: &str| kind_ok_table(table, edge, sym_kind);
        let ctx = BinderContext {
            file_ctx,
            ref_ctx,
            lookup,
            kind: &kind,
            profile,
        };
        self.engine.bind(&ctx)
    }
}

/// `true` when the chain's root segment names a namespace/module declaration
/// (`React` in `React.useState`). Namespace-qualified member access roots on a
/// namespace the chain walker can't type as a value, so a declined chain with a
/// namespace root falls through to the bare-name ladder — scoped by the ref's
/// module — which binds the member under the imported namespace.
fn chain_root_is_namespace(chain: &crate::types::MemberChain, lookup: &dyn SymbolLookup) -> bool {
    let Some(root) = chain.segments.first() else {
        return false;
    };
    lookup
        .by_name(&root.name)
        .iter()
        .any(|s| matches!(s.kind.as_str(), "namespace" | "module"))
}

/// Profile-table-driven kind compatibility. An unrecognised symbol-kind string
/// defaults permissive so an extractor typo doesn't silently hide a real symbol.
fn kind_ok_table(table: KindTable, edge: EdgeKind, sym_kind: &str) -> bool {
    match SymbolKind::from_str(sym_kind) {
        Ok(parsed) => KindCompatibility::check(table, edge, parsed),
        Err(_) => true,
    }
}

/// Test-only re-export of the engine's kind-compatibility predicate so sibling
/// tests can assert which symbol kinds a profile's `KindTable` admits for an
/// edge — the exact gate the rule ladder consults via `BinderContext.kind`.
#[cfg(test)]
pub(super) fn kind_ok_table_for_test(table: KindTable, edge: EdgeKind, sym_kind: &str) -> bool {
    kind_ok_table(table, edge, sym_kind)
}

#[cfg(test)]
#[path = "semantic_model_tests.rs"]
mod tests;
