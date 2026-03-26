pub mod bash;
pub mod c_lang;
pub mod cpp;
pub mod csharp;
pub mod dart;
pub mod elixir;
pub mod generic;
pub mod go;
pub mod java;
pub mod javascript;
pub mod kotlin;
pub mod php;
pub mod python;
pub mod ruby;
pub mod rust;
pub mod scala;
pub mod swift;
pub mod typescript;

use crate::types::{ExtractedRef, ExtractedSymbol};

/// Shared extraction result for the newer extractors (bash, c_lang, dart, etc.)
/// that do not define a per-language result type.
pub struct ExtractionResult {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub has_errors: bool,
}

impl ExtractionResult {
    pub fn new(
        symbols: Vec<ExtractedSymbol>,
        refs: Vec<ExtractedRef>,
        has_errors: bool,
    ) -> Self {
        Self { symbols, refs, has_errors }
    }
}
