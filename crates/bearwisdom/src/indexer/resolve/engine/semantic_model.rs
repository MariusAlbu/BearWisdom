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

use crate::indexer::resolve::engine::cause::{Cause, CauseKind};
use crate::indexer::resolve::engine::contract::{FileContext, RefContext, SymbolInfo, SymbolLookup};
use crate::type_checker::profile::language_profile::{
    KindCompatibility, KindTable, LanguageProfile,
};
use crate::types::{EdgeKind, SymbolKind};

use super::{BindOutcome, BinderContext, Binder};

/// Outcome of solving one ref, chain-bearing or chain-less.
pub enum SolveOutcome {
    /// A rule or the chain walk bound the ref.
    Resolved(SymbolInfo),
    /// Nothing bound it — an honest miss, carrying the first-uncaptured-type
    /// cause when a death site could attribute one.
    Unresolved(Option<Cause>),
    /// The rule ladder declined the ref as a known non-project construct
    /// (`LanguageProfile::builtin_skip`) rather than a missing project symbol.
    Drained,
}

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
    ) -> SolveOutcome {
        if let Some(chain) = ref_ctx.extracted_ref.chain.as_ref() {
            match super::chain::bind_member_access(ref_ctx, file_ctx, lookup) {
                Ok(res) => return SolveOutcome::Resolved(res),
                Err(cause) => {
                    // A multi-segment chain the walk declined is normally a genuine
                    // miss: a same-named sibling must not hijack `a.b.c`. Two
                    // exceptions both root on a MODULE the chain walker can't type
                    // as a value, so the bare-name ladder resolves the member under
                    // that module (scoped, so it can't bind an unrelated sibling):
                    //   - a namespace/module SYMBOL root (`React.useState`);
                    //   - a wildcard/namespace IMPORT root (`import * as v from
                    //     'm'; v.x`) — the alias names no value, and the member is a
                    //     module export.
                    // A single-segment "chain" carries no receiver and always falls
                    // through.
                    if chain.segments.len() > 1
                        && !chain_root_is_namespace(chain, lookup)
                        && !chain_root_is_wildcard_import(chain, file_ctx)
                    {
                        return SolveOutcome::Unresolved(cause);
                    }
                }
            }
        }
        match self.resolve_chain_less(ref_ctx, file_ctx, lookup, profile) {
            BindOutcome::Resolved(res, _rule) => SolveOutcome::Resolved(res),
            BindOutcome::Unresolved => {
                SolveOutcome::Unresolved(Some(Cause::new(None, CauseKind::UnboundRoot)))
            }
            BindOutcome::Drained => SolveOutcome::Drained,
        }
    }

    /// Solve one chain-less ref through the rule ladder. Builds the profile kind
    /// predicate, assembles a `BinderContext`, and runs the rules. See
    /// [`BindOutcome`].
    pub fn resolve_chain_less(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
    ) -> BindOutcome {
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

/// `true` when the chain's root segment names a wildcard/namespace import in this
/// file (`import * as v from 'm'` — `is_wildcard`, matched by alias or imported
/// name). The alias names a module, not a value, so `v.member` resolves against
/// the module's exports through the bare-name ladder rather than the value walk.
fn chain_root_is_wildcard_import(
    chain: &crate::types::MemberChain,
    file_ctx: &FileContext,
) -> bool {
    let Some(root) = chain.segments.first() else {
        return false;
    };
    file_ctx.imports.iter().any(|i| {
        i.is_wildcard
            && (i.alias.as_deref() == Some(root.name.as_str()) || i.imported_name == root.name)
    })
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
