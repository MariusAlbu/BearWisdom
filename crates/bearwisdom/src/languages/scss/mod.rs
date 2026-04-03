//! SCSS language plugin.
//!
//! Uses the CSS tree-sitter grammar, which parses the SCSS superset well enough
//! for structural extraction. Dedicated SCSS grammar (tree-sitter-scss) is not
//! in the workspace; the CSS grammar handles the core constructs.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct ScssPlugin;

impl LanguagePlugin for ScssPlugin {
    fn id(&self) -> &str {
        "scss"
    }

    fn language_ids(&self) -> &[&str] {
        &["scss", "sass"]
    }

    fn extensions(&self) -> &[&str] {
        &[".scss", ".sass"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        // CSS grammar parses SCSS syntax for the constructs we care about.
        Some(tree_sitter_css::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source, file_path)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "mixin_statement",
            "function_statement",
            "rule_set",
            "keyframes_statement",
            "placeholder",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "include_statement",
            "extend_statement",
            "use_statement",
            "forward_statement",
            "import_statement",
            "call_expression",
        ]
    }
}
