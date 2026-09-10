//! javascript language plugin.

mod calls;
pub(crate) mod chains;
pub(crate) mod component_tags;
pub mod extract;
pub(crate) mod flow;
mod globals;
mod helpers;
mod imports;
pub(crate) mod keywords;
pub(crate) mod predicates;
pub(crate) mod profile;

pub use profile::JAVASCRIPT_PROFILE;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod calls_tests;

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod predicates_tests;

#[cfg(test)]
#[path = "flow_tests.rs"]
mod flow_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct JavascriptPlugin;

impl LanguagePlugin for JavascriptPlugin {
    fn id(&self) -> &str {
        "javascript"
    }

    fn language_ids(&self) -> &[&str] {
        &["javascript", "jsx"]
    }

    fn extensions(&self) -> &[&str] {
        &[".js", ".jsx", ".mjs", ".cjs"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_javascript::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = lang_id;
        // Vendored library bundles (jQuery, typeahead.js, generated theme
        // builds, etc.) produce tens of thousands of unresolved refs that
        // aren't first-party code. Mirrors the HTML auto-generated docs
        // skip in `languages::html::extract`. See `looks_vendored_bundle`
        // for the detection signals.
        if extract::looks_vendored_bundle(source, file_path) {
            return ExtractionResult::empty();
        }
        let mut result = extract::extract(source);
        crate::languages::common::append_ember_helper_default_export(
            file_path,
            source,
            &mut result,
        );
        crate::languages::common::append_handlebars_register_helper_globals(source, &mut result);
        crate::languages::common::append_amd_define_imports(source, &mut result);
        crate::languages::common::append_jquery_fn_plugin_globals(source, &mut result);
        result
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "class",
            "function_declaration",
            "generator_function_declaration",
            // `function_expression` is intentionally omitted: standalone function
            // expressions (as callbacks, IIFEs, object property values) have no
            // extractable name. Named cases like `const f = function() {}` are
            // already captured under the parent `lexical_declaration` node.
            "method_definition",
            "variable_declaration",
            "lexical_declaration",
            "field_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "new_expression",
            "import_statement",
            "export_statement",
            "class_heritage",
            "jsx_opening_element",
            "jsx_self_closing_element",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn primitive_member_head(&self, head: &str) -> Option<String> {
        match head {
            "string" => Some("String".to_string()),
            "number" => Some("Number".to_string()),
            "bigint" => Some("BigInt".to_string()),
            "boolean" => Some("Boolean".to_string()),
            "symbol" => Some("Symbol".to_string()),
            _ => None,
        }
    }

    fn has_homogeneous_computed_access(&self, head: &str) -> bool {
        head == "Array"
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::JAVASCRIPT_PROFILE)
    }

    fn component_tag_head<'a>(&self, target: &'a str) -> Option<&'a str> {
        component_tags::component_tag_head(target)
    }

    fn is_component_file(&self, path: &str) -> bool {
        component_tags::is_component_file(path)
    }

    fn component_selectors(
        &self,
        source: &str,
        symbols: &[crate::types::ExtractedSymbol],
    ) -> Vec<(String, String)> {
        crate::languages::typescript::selectors::extract_custom_element_defines(source, symbols)
    }

    fn plugin_flow_emissions(
        &self,
        source: &str,
        _file_path: &str,
    ) -> Vec<(u32, crate::indexer::resolve::flow_emit::FlowEmission)> {
        crate::languages::typescript::connectors::extract_typescript_graphql(source)
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::JS_FLOW_CONFIG)
    }
}
