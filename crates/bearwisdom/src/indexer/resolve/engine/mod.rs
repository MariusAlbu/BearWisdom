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
// react-tanstack-query (a differential gate), then the remaining gap to a 99%
// resolution rate is closed by adding/fixing one rule at a time.
// =============================================================================

use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{EdgeKind, ExtractedRef};

use crate::indexer::resolve::engine::contract::{
    FileContext, RefContext, SymbolInfo, SymbolLookup, RESOLVED_CONFIDENCE,
};

pub(crate) mod contract;
pub mod alias;
pub mod chain;
pub mod semantic_model;
pub mod pipeline;
pub mod rules;
pub mod support;
pub mod compilation;
pub mod module_identity;
pub mod trace;

#[cfg(test)]
pub(crate) mod testkit;

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
    #[inline]
    pub fn target(&self) -> &str {
        self.ref_ctx.extracted_ref.target_name.as_str()
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

    /// Resolve one ref. Returns `(resolution, rule_name)`, or `None` when the
    /// ladder ran out or a rule stopped it — an honestly-unresolved ref,
    /// attributable to a missing or buggy rule, not to the engine.
    pub fn bind(&self, ctx: &BinderContext) -> Option<(SymbolInfo, &'static str)> {
        for rule in &self.rules {
            match rule.apply(ctx) {
                LookupResult::Resolved(res) => return Some((res, rule.name())),
                LookupResult::Pass => continue,
                LookupResult::Stop => return None,
            }
        }
        None
    }
}
