// =============================================================================
// indexer/resolve/engine/registry.rs — engine wrapper
//
// The legacy `LanguageResolver` trait has been deleted. Per-language resolution
// flows entirely through `LanguageEngineHooks`. This file now contains only
// the vestigial `ResolutionEngine` struct that the resolve-loop wires through;
// it carries no state and the resolution dispatch happens in
// `type_checker::engine::Engine`.
// =============================================================================

/// Vestigial dispatcher kept for ABI compatibility; resolution flows entirely
/// through `LanguageEngineHooks`. The struct holds no state.
pub struct ResolutionEngine;

impl ResolutionEngine {
    pub fn new() -> Self {
        Self
    }
}
