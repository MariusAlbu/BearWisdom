//! Pinned TypeScript/TSX grammar with source-shape repairs for ambient providers.
//! Generated parsers are checked in; Cargo does not run npm or fetch grammars.

use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_typescript() -> *const ();
    fn tree_sitter_tsx() -> *const ();
}

pub const LANGUAGE_TYPESCRIPT: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_typescript) };
pub const LANGUAGE_TSX: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_tsx) };
pub const TYPESCRIPT_NODE_TYPES: &str = include_str!("typescript/src/node-types.json");
pub const TSX_NODE_TYPES: &str = include_str!("tsx/src/node-types.json");
pub const HIGHLIGHTS_QUERY: &str = include_str!("queries/highlights.scm");
pub const LOCALS_QUERY: &str = include_str!("queries/locals.scm");
pub const TAGS_QUERY: &str = include_str!("queries/tags.scm");

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
