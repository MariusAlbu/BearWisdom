// =============================================================================
// type_checker/profile/hooks.rs — LanguageEngineHooks trait
//
// Escape hatch for per-language behaviour that can't be expressed as
// LanguageProfile data: reshaping refs whose syntactic form doesn't match
// the canonical contract (ObjC bracket calls), synthesising members the
// runtime adds at decorator time (Python @dataclass, TS @Component, Java
// @Entity), enriching external types with information not present in their
// source, special dispatch (Haskell typeclass, R S4), custom flow emission.
//
// All methods have no-op defaults. The default impl `NoOpHooks` is what the
// engine binds when a language plugin does not ship its own hooks.
// =============================================================================

use crate::types::ExtractedRef;

use super::super::core::types::{TypeArena, TypeId};

/// Per-resolution context the dispatch hook can inspect to pick among
/// candidates.
#[derive(Debug)]
pub struct DispatchContext<'a> {
    /// The unresolved Calls ref the engine is dispatching.
    pub call_ref: &'a ExtractedRef,
    /// Argument TypeIds inferred so far (positional). Empty when the engine
    /// hasn't inferred any.
    pub arg_types: &'a [TypeId],
    /// Expected return type from the call site (None when free-standing).
    pub expected_return: Option<TypeId>,
}

/// Per-ref context the flow-emission hook can inspect when deciding whether
/// to emit a cross-tier `FlowEmission`.
#[derive(Debug)]
pub struct RefContext<'a> {
    /// Canonical file path the ref lives in.
    pub file_path: &'a str,
    /// Language id (`ParsedFile.language`).
    pub language: &'a str,
    /// Owning symbol's qualified_name.
    pub owner_qname: Option<&'a str>,
}

/// Minimal description of a synthesized member. The engine inserts it as if
/// the source had declared it directly.
#[derive(Debug, Clone)]
pub struct SynthesizedMember {
    pub name: String,
    pub kind: crate::types::SymbolKind,
    pub return_type: Option<TypeId>,
    pub param_types: Vec<TypeId>,
}

/// Stand-in target for `resolve_dispatch_special`. The hook returns the DB
/// symbol id of the implementation to bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchTarget {
    pub symbol_id: i64,
}

/// Outcome surfaced by `detect_flow_emission_special`. The engine treats it
/// the same way it treats flow emissions surfaced by chain rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomFlowEmission {
    pub kind: &'static str,
    pub target_qname: Option<String>,
    pub protocol: Option<&'static str>,
    pub confidence: u8,
}

/// Behaviour a language plugin may override beyond what `LanguageProfile`
/// expresses as data. All methods have no-op defaults; outliers override
/// only the methods they need.
pub trait LanguageEngineHooks: Send + Sync {
    /// Reshape an `ExtractedRef` whose syntactic form differs from the
    /// canonical contract. ObjC's `[obj msg:arg]` flattens to chain form
    /// via this hook.
    fn preprocess_ref(&self, _ref_: &mut ExtractedRef) {}

    /// Synthesize members a class gains at runtime (Python `@dataclass`,
    /// TypeScript `@Component`, Java `@Entity`). Default: nothing
    /// synthesised.
    fn synthesize_members(
        &self,
        _class_qname: &str,
        _decorators: &[String],
        _arena: &mut TypeArena,
    ) -> Vec<SynthesizedMember> {
        Vec::new()
    }

    /// Enrich an externally-sourced type with detail not present in its
    /// declaration (Rust associated-type bindings, TypeScript declaration
    /// merging). Default: no-op.
    fn enrich_external_type(&self, _ty: TypeId, _arena: &mut TypeArena) {}

    /// Pick a dispatch target for non-receiver dispatch axes. Returns the
    /// DB symbol id of the chosen target, or `None` when the hook can't
    /// decide (engine then falls back to receiver dispatch).
    fn resolve_dispatch_special(
        &self,
        _candidate_symbol_ids: &[i64],
        _ctx: &DispatchContext<'_>,
    ) -> Option<DispatchTarget> {
        None
    }

    /// Detect a cross-tier flow emission the chain-walker rules can't
    /// express. Default: no emission.
    fn detect_flow_emission_special(
        &self,
        _ref_: &ExtractedRef,
        _ctx: &RefContext<'_>,
    ) -> Option<CustomFlowEmission> {
        None
    }
}

/// Concrete no-op implementation. Bound by the engine when a language
/// plugin does not ship its own hooks.
pub struct NoOpHooks;

impl LanguageEngineHooks for NoOpHooks {}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
