// =============================================================================
// parser/languages.rs — grammar loader + extractor dispatch
//
// `get_language` (language-id → tree-sitter Language) is re-exported from the
// shared `code-grammars` crate so the indexer and editor frontends share one
// grammar registry. `has_extractor` stays here — it is indexer dispatch, not a
// grammar concern.
// =============================================================================

pub use code_grammars::get_language;

/// Returns `true` if the language has a full dedicated symbol extractor,
/// not just grammar-based generic extraction.
///
/// Used by the indexer to decide whether to run a specialised extractor or
/// fall back to the generic DFS walker.
pub fn has_extractor(lang: &str) -> bool {
    matches!(
        lang,
        "csharp" | "typescript" | "tsx" | "rust" | "python" | "go" | "java"
    )
}

#[cfg(test)]
#[path = "languages_tests.rs"]
mod tests;
