//! Razor host-level extraction.
//!
//! Razor has no tree-sitter grammar in this workspace — every Razor
//! construct (`@{...}`, `@code{...}`, `@functions{...}`, `@(expr)`,
//! `@model`, `@inject`, `@using`, `@inherits`, `@implements`,
//! `@namespace`, `@if`/`@foreach`/`@while`/`@switch`/`@for`, and
//! `<script>` blocks) is handled by the embedded-region pipeline in
//! `embedded::detect_regions`. The host extractor itself produces no
//! symbols or refs.

use crate::types::ExtractionResult;

pub fn extract(_source: &str, _file_path: &str) -> ExtractionResult {
    ExtractionResult::new(Vec::new(), Vec::new(), false)
}
