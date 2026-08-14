// =============================================================================
// indexer/resolve/engine — rule-based resolution engine
//
// The engine island. It owns its resolution vocabulary in `contract` (the data
// types, the SymbolLookup trait, the stateless parse helpers) and depends on
// nothing under the frozen `legacy/`. Each ref is bound by one `LookupRule` —
// each in its own file, self-contained, unit-tested — instead of a monolithic
// strategy tower.
//
// Why: a ref that no rule resolves is honestly unresolved, and because every
// rule is isolated, an unresolved ref is diagnosable as either a bug in a
// specific named rule or a missing rule — never a needle in a monolith.
//
// Each rule file carries the code needed to understand it at a glance: the
// helper logic only that rule uses is copied inline; only code that genuinely
// repeats across rules lives in `support`.
//
// Validation: the new engine reproduces the old ladder's resolutions exactly on
// a differential-gate fixture, then the remaining gap to a 99% resolution rate is
// closed by adding/fixing one rule at a time.
// =============================================================================

use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{EdgeKind, ExtractedRef};

use crate::indexer::resolve::engine::contract::{
    FileContext, RefContext, SymbolInfo, SymbolLookup, RESOLVED_CONFIDENCE,
};

pub(crate) mod contract;
pub mod alias;
pub mod arg_types;
pub mod cause;
pub mod chain;
pub mod chain_root;
pub mod demand_veto;
pub mod ext_lang_visibility;
pub mod extension_method;
pub mod externals_demand;
mod file_context;
mod file_lookup;
mod flush;
pub mod generic_shadow;
pub mod generics;
pub mod head_decl;
pub mod implicit_root;
pub mod import_qualify;
pub mod lambda_seed;
pub mod mapped_members;
pub mod module_augmentation;
pub mod overload_alts;
pub mod relative_imports;
pub mod segment_args;
pub mod substitution;
pub mod type_mention_demand;
mod module_scheme;
mod parallel_pass;
mod parent_resolution;
mod root_import_discipline;
mod unbound_cause;
pub mod semantic_model;
pub mod pipeline;
mod tree_build;
pub mod rules;
pub mod path_match;
pub mod reexports;
mod reexports_candidates;
mod module_specifier;
pub mod support;
pub mod compilation;
mod compilation_persist;
pub mod composite_members;
pub mod module_identity;
pub mod trace;

#[cfg(test)]
pub(crate) mod testkit;
#[cfg(test)]
pub(crate) mod testkit_fixtures;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

/// Everything a rule needs to resolve one ref. Borrowed; built fresh per ref by
/// the engine. Mirrors the inputs the old `DefaultResolver` strategies read —
/// the file's imports/namespace, the ref + its scope chain, the symbol index,
/// the candidate-kind predicate, and the language profile data — so a strategy
/// body lifts into a rule with no behavioural change.
pub struct BinderContext<'a> {
    /// File-level context: imports, file namespace, file path, language.
    pub file_ctx: &'a FileContext,
    /// The ref being resolved, plus its source symbol and scope chain.
    pub ref_ctx: &'a RefContext<'a>,
    /// The symbol index — internal symbols plus lazy external materialization.
    pub lookup: &'a dyn SymbolLookup,
    /// Candidate-kind compatibility predicate. Built by the engine from the
    /// profile's kind-compatibility table; `|_, _| true` accepts any kind.
    pub kind: &'a dyn Fn(EdgeKind, &str) -> bool,
    /// The language profile — the per-language data the rules read (separators,
    /// self keywords, name normalization, gates).
    pub profile: &'a LanguageProfile,
}

impl<'a> BinderContext<'a> {
    /// The ref being resolved.
    #[inline]
    pub fn r(&self) -> &ExtractedRef {
        self.ref_ctx.extracted_ref
    }

    /// The ref's target name.
    ///
    /// For nominal-binding edge kinds (Inherits, Implements, TypeRef,
    /// Instantiates) returns the bare head of the generic application:
    /// `QueryObserverBaseResult<TData, TError>` → `QueryObserverBaseResult`.
    /// No-op for non-generic targets (no `<` or `[` → whole string returned).
    /// Other edge kinds return the raw target_name unchanged.
    #[inline]
    pub fn target(&self) -> &str {
        let raw = self.ref_ctx.extracted_ref.target_name.as_str();
        match self.ref_ctx.extracted_ref.kind {
            EdgeKind::Inherits
            | EdgeKind::Implements
            | EdgeKind::TypeRef
            | EdgeKind::Instantiates => {
                crate::indexer::resolve::engine::contract::chain_walker::parse_type_head_and_args(
                    raw,
                )
                .0
            }
            _ => raw,
        }
    }

    /// The ref's edge kind.
    #[inline]
    pub fn edge_kind(&self) -> EdgeKind {
        self.ref_ctx.extracted_ref.kind
    }

    /// Build a successful resolution for `target_symbol_id`, tagged with the
    /// rule's `strategy` name. SymbolInfo is binary — confidence is always
    /// `RESOLVED_CONFIDENCE`; a rule that can't bind returns `Pass`/`Stop`.
    #[inline]
    pub fn resolved(&self, target_symbol_id: i64, strategy: &'static str) -> SymbolInfo {
        SymbolInfo {
            target_symbol_id,
            confidence: RESOLVED_CONFIDENCE,
            strategy,
            resolved_yield_type: None,
            flow_emit: None,
        }
    }
}

/// One rule's verdict on a ref.
#[derive(Debug)]
pub enum LookupResult {
    /// The rule bound the ref — first match wins; the ladder stops here.
    Resolved(SymbolInfo),
    /// The rule declined; try the next rule.
    Pass,
    /// The rule declined AND ends the ladder: the ref is honestly unresolved and
    /// no later rule may bind it (a module-decline / terminal-anchor guard, so a
    /// same-named local cannot hijack an external prefix).
    Stop,
    /// The rule declined AND ends the ladder because the target names a
    /// language builtin or other non-project construct
    /// (`LanguageProfile::builtin_skip`), not a missing project symbol. Distinct
    /// from `Stop` so the caller can write the ref to `unresolved_refs` tagged
    /// `drained=1` and exclude it from the resolution-rate denominator.
    Drained,
}

/// One resolution case. Single responsibility: examine the context and either
/// bind the ref, pass to the next rule, or stop the ladder. Rules never mutate;
/// they read `ctx` and return a verdict.
pub trait LookupRule: Send + Sync {
    /// Stable id, surfaced in diagnostics ("resolved by `qname_exact`" /
    /// "no rule matched"). Short and unique.
    fn name(&self) -> &'static str;

    /// Apply the rule to `ctx`.
    fn apply(&self, ctx: &BinderContext) -> LookupResult;
}

/// Outcome of running the full ladder against one ref.
pub enum BindOutcome {
    /// A rule bound the ref, tagged with the rule's name for diagnostics.
    Resolved(SymbolInfo, &'static str),
    /// No rule bound it and none marked it a non-project construct — an
    /// honest miss, attributable to a missing or buggy rule.
    Unresolved,
    /// A rule declined the ref as a known non-project construct
    /// (`LookupResult::Drained`) before the binding rungs ran.
    Drained,
}

/// An ordered set of rules. First rule that resolves wins; the winning rule's
/// name rides along for diagnostics. A `Stop` ends the ladder with no binding.
pub struct Binder {
    rules: Vec<Box<dyn LookupRule>>,
}

impl Binder {
    pub fn new(rules: Vec<Box<dyn LookupRule>>) -> Self {
        Self { rules }
    }

    /// The production rule set, in canonical ladder order.
    pub fn production() -> Self {
        Self::new(rules::default_rules())
    }

    /// The module-evidence subset — the rungs a declined member chain with an
    /// extractor-set `module` may still run. See `rules::module_evidence_rules`.
    pub fn module_evidence() -> Self {
        Self::new(rules::module_evidence_rules())
    }

    /// Resolve one ref against the ladder. See [`BindOutcome`].
    pub fn bind(&self, ctx: &BinderContext) -> BindOutcome {
        for rule in &self.rules {
            match rule.apply(ctx) {
                LookupResult::Resolved(res) => return BindOutcome::Resolved(res, rule.name()),
                LookupResult::Pass => continue,
                LookupResult::Stop => return BindOutcome::Unresolved,
                LookupResult::Drained => return BindOutcome::Drained,
            }
        }
        BindOutcome::Unresolved
    }
}
