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
    /// Module-evidence subset for a declined member chain that carries an
    /// extractor-set `module` — every rung in it is scoped by that module, so
    /// the fall-through can never bind an unrelated same-named sibling.
    module_engine: Binder,
}

impl SemanticModel {
    /// Build a solver over the production rule ladder.
    pub fn production() -> Self {
        Self {
            engine: Binder::production(),
            module_engine: Binder::module_evidence(),
        }
    }

    /// Build a solver over an explicit rule set (tests inject a subset).
    pub fn new(engine: Binder) -> Self {
        Self {
            engine,
            module_engine: Binder::module_evidence(),
        }
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
            match super::chain::bind_member_access(ref_ctx, file_ctx, lookup, profile) {
                Ok(res) => return SolveOutcome::Resolved(res),
                Err(cause) => {
                    // A multi-segment chain the walk declined is normally a genuine
                    // miss: a same-named sibling must not hijack `a.b.c`. Three
                    // exceptions all root on a MODULE the chain walker can't type
                    // as a value, so a scoped ladder resolves the member under
                    // that module:
                    //   - a namespace/module SYMBOL root (`React.useState`) and a
                    //     wildcard/namespace IMPORT root (`import * as v; v.x`)
                    //     run the full bare-name ladder, whose import rungs carry
                    //     the scoping;
                    //   - a ref whose `module` IS the chain's own qualifier path
                    //     (a qualified `mod::path::f()` call — the target is a
                    //     direct member of the module) runs ONLY the
                    //     module-evidence rungs. Every probe is scoped by the
                    //     module, so no ambient / same-file / global rung can
                    //     bind an unrelated same-named sibling. A module tag that
                    //     merely names where the chain's ROOT was imported from
                    //     (`client.get()` tagged with the client's package) says
                    //     nothing about the member's home and does not qualify.
                    // A single-segment "chain" carries no receiver and always falls
                    // through.
                    if chain.segments.len() > 1
                        && !chain_root_is_namespace(chain, lookup)
                        && !chain_root_is_wildcard_import(chain, file_ctx)
                    {
                        if module_is_chain_qualifier(ref_ctx.extracted_ref, chain, profile) {
                            return match self.resolve_module_scoped(
                                ref_ctx, file_ctx, lookup, profile,
                            ) {
                                BindOutcome::Resolved(res, _rule) => SolveOutcome::Resolved(res),
                                BindOutcome::Drained => SolveOutcome::Drained,
                                BindOutcome::Unresolved => SolveOutcome::Unresolved(cause),
                            };
                        }
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

    /// Solve a declined module-tagged member chain through the module-evidence
    /// rung subset. Same context assembly as `resolve_chain_less`; only the
    /// rule set differs.
    fn resolve_module_scoped(
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
        self.module_engine.bind(&ctx)
    }
}

/// `true` when the ref's extractor-set `module` is exactly the chain's own
/// qualifier path — every segment but the target, joined by the profile's
/// separator (or the universal `.`). That shape means the target is a DIRECT
/// member of the module (`serde_json::from_value` → module `serde_json`,
/// chain `[serde_json, from_value]`), so the module-evidence rungs can bind
/// it. A module tag naming where the chain's root was imported from joins to
/// a different string and is rejected.
fn module_is_chain_qualifier(
    r: &crate::types::ExtractedRef,
    chain: &crate::types::MemberChain,
    profile: &LanguageProfile,
) -> bool {
    let Some(module) = r.module.as_deref() else {
        return false;
    };
    let quals = &chain.segments[..chain.segments.len() - 1];
    let mut for_sep = |sep: &str| {
        let joined = quals.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(sep);
        joined == module
    };
    for_sep(profile.qname_separator) || for_sep(".")
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
