//! Generic fallback extractor — works for any language with a tree-sitter grammar.

mod helpers;
pub mod extract;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;
