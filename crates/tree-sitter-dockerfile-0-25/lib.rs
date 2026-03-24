//! Dockerfile grammar for tree-sitter, compatible with tree-sitter 0.25+.
//!
//! This is a local wrapper around the tree-sitter-dockerfile 0.2.0 C parser
//! source, re-exported with the modern `LanguageFn` API.

use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_dockerfile() -> *const ();
}

/// The tree-sitter [`LanguageFn`] for Dockerfile.
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_dockerfile) };

/// The content of the `node-types.json` file for this grammar.
pub const NODE_TYPES: &str = include_str!("src/node-types.json");

#[cfg(test)]
mod tests {
    #[test]
    fn test_can_load_grammar() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::LANGUAGE.into())
            .expect("Error loading Dockerfile parser");
    }
}
