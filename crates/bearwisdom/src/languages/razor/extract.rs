//! Razor host-level extraction.
//!
//! Razor has no tree-sitter grammar in this workspace — the interesting
//! content lives inside `@{...}`, `@code{...}`, `@functions{...}`,
//! `@(expr)` and `<script>` blocks, all of which are handled by the
//! embedded-region pipeline (see `embedded::detect_regions`). The host
//! extractor itself produces no symbols or refs.
//!
//! Future work: surface `@model Foo` / `@inject Foo` / `@using X` /
//! `@layout _Layout` as direct type or namespace refs. The MVP skips
//! these because their rest-of-line payload isn't a valid C# compilation
//! unit (a bare type name), so routing through the embedded C# pipeline
//! produces no usable output.

use crate::types::ExtractionResult;

pub fn extract(_source: &str, _file_path: &str) -> ExtractionResult {
    ExtractionResult::new(Vec::new(), Vec::new(), false)
}
