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

use crate::indexer::resolve::engine::cause::{Cause, CauseKind};
use crate::indexer::resolve::engine::contract::{
    FileContext, RefContext, SymbolInfo, SymbolLookup,
};
use crate::type_checker::profile::language_profile::{KindTable, LanguageProfile};
use crate::types::EdgeKind;

use super::{BindOutcome, Binder, BinderContext};

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
        let reference = ref_ctx.extracted_ref;
        if let Some(outcome) = lexical::bind_lexical_call(ref_ctx, file_ctx, lookup) {
            return outcome;
        }
        if let Some(outcome) = lexical::bind_lexical_import_binding(ref_ctx, lookup) {
            return outcome;
        }
        // The walk's own diagnosis of a declined chain. When a namespace or
        // wildcard-import root sends the chain through the module-scoped ladder
        // below and that ladder also comes up empty, this outranks a bare-name
        // classification of the last segment: the root that never linked or
        // the member the receiver lacks is the cause, not the leaf's name.
        let mut chain_cause: Option<Cause> = None;
        if let Some(chain) = ref_ctx.extracted_ref.chain.as_ref() {
            match super::chain::bind_member_access(ref_ctx, file_ctx, lookup, profile) {
                Ok(res) => return SolveOutcome::Resolved(res),
                Err(cause) => {
                    chain_cause = cause;
                    if chain
                        .segments
                        .first()
                        .is_some_and(|s| s.kind == crate::types::SegmentKind::BaseRef)
                    {
                        return SolveOutcome::Unresolved(cause);
                    }
                    if chain.segments.len() > 1
                        && lookup.local_reference(reference.byte_offset).is_some()
                    {
                        return SolveOutcome::Unresolved(cause);
                    }
                    // A multi-segment chain the walk declined is normally a genuine
                    // miss: a same-named sibling must not hijack `a.b.c`. The only
                    // exception is a root whose module is proven at THIS source
                    // site: an imported/ambient/same-package namespace, a wildcard
                    // import alias, or a ref whose own `module` is its qualifier.
                    // Re-run only the module-evidence subset with those source-
                    // addressed module paths; never the full bare-name ladder.
                    if chain.segments.len() > 1 {
                        let mut modules = namespace_root_modules(
                            chain,
                            file_ctx,
                            ref_ctx.file_package_id,
                            lookup,
                            profile,
                        );
                        for module in wildcard_root_modules(chain, file_ctx) {
                            push_unique_module(&mut modules, &module);
                        }
                        if module_is_chain_qualifier(ref_ctx.extracted_ref, chain, profile) {
                            if let Some(module) = ref_ctx.extracted_ref.module.as_deref() {
                                push_unique_module(&mut modules, module);
                            }
                        }

                        // A declined walk without a cause of its own anchored the
                        // root (a failed anchor always carries one) and died on a
                        // later hop silently — record that as the chain's cause.
                        let cause = cause.or(Some(Cause::new(None, CauseKind::ChainDeclined)));
                        if !modules.is_empty() {
                            return match self.resolve_module_scoped_at(
                                ref_ctx, file_ctx, lookup, profile, &modules,
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
            BindOutcome::Unresolved => SolveOutcome::Unresolved(chain_cause.or_else(|| {
                Some(super::unbound_cause::classify_unbound_root(
                    &ref_ctx.extracted_ref.target_name,
                    &ref_ctx.scope_chain,
                    file_ctx,
                    lookup,
                    ref_ctx.file_package_id,
                ))
            })),
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

    /// Run a declined namespace/wildcard chain only through modules the source
    /// site proves. Each attempt gets a synthetic `ref.module`, so the existing
    /// module-evidence rules remain the sole binding surface.
    fn resolve_module_scoped_at(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
        modules: &[String],
    ) -> BindOutcome {
        for module in modules {
            let mut scoped_ref = ref_ctx.extracted_ref.clone();
            scoped_ref.module = Some(module.clone());
            let scoped_ctx = RefContext {
                extracted_ref: &scoped_ref,
                source_symbol: ref_ctx.source_symbol,
                scope_chain: ref_ctx.scope_chain.clone(),
                file_package_id: ref_ctx.file_package_id,
                source_symbol_id: ref_ctx.source_symbol_id,
            };
            match self.resolve_module_scoped(&scoped_ctx, file_ctx, lookup, profile) {
                BindOutcome::Resolved(res, rule) => return BindOutcome::Resolved(res, rule),
                BindOutcome::Drained => return BindOutcome::Drained,
                BindOutcome::Unresolved => {}
            }
        }
        BindOutcome::Unresolved
    }
}

/// Profile-table-driven kind compatibility. An unrecognised symbol-kind string
/// defaults permissive so an extractor typo doesn't silently hide a real symbol.
fn kind_ok_table(table: KindTable, edge: EdgeKind, sym_kind: &str) -> bool {
    crate::type_checker::profile::chain_specs::kind_ok(table, edge, sym_kind)
}

/// Test-only re-export of the engine's kind-compatibility predicate so sibling
/// tests can assert which symbol kinds a profile's `KindTable` admits for an
/// edge — the exact gate the rule ladder consults via `BinderContext.kind`.
#[cfg(test)]
pub(super) fn kind_ok_table_for_test(table: KindTable, edge: EdgeKind, sym_kind: &str) -> bool {
    kind_ok_table(table, edge, sym_kind)
}

#[path = "semantic_model_lexical.rs"]
mod lexical;
#[path = "semantic_model_scoping.rs"]
mod scoping;
use scoping::*;

#[cfg(test)]
#[path = "semantic_model_tests.rs"]
mod tests;
