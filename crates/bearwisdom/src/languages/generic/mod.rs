//! Generic fallback extractor — works for any language with a tree-sitter grammar.

pub mod extract;
mod helpers;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;
