//! Angular template language plugin.
//!
//! Angular templates (.html files inside Angular projects) are parsed using the
//! HTML grammar as a structural fallback. The dedicated tree-sitter-angular
//! grammar is not in the workspace; HTML provides enough structure to extract
//! component selector usages, pipe calls, and event handler bindings.
//!
//! Angular templates do not define symbols. All edges are Calls from a
//! sentinel "template" symbol at index 0 to referenced components, pipes,
//! and handler methods.
//!
//! Note: Angular .html template files cannot be distinguished from plain HTML
//! by extension alone. The `language_ids` registration routes files that the
//! profile detector has already tagged as "angular" (based on proximity to
//! angular.json or @angular/* deps in package.json).

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct AngularPlugin;

impl LanguagePlugin for AngularPlugin {
    fn id(&self) -> &str {
        "angular"
    }

    fn language_ids(&self) -> &[&str] {
        &["angular"]
    }

    fn extensions(&self) -> &[&str] {
        // Angular templates share the .html extension; detection is by lang_id.
        &[".component.html"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_html::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source, file_path)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        // Angular templates define no symbols of their own.
        &[]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        // Component selector tags and pipe calls produce Calls edges.
        &[
            "element",
            "self_closing_tag",
            "pipe_call",
            "call_expression",
            "interpolation",
            "property_binding",
            "event_binding",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }
}
