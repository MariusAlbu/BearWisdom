//! scala language plugin.
mod callback_lexical;

mod calls;
pub(crate) mod decorators;
pub mod extract;
pub(crate) mod flow;
mod helpers;
pub(crate) mod keywords;
mod predicates;
pub(crate) mod profile;
mod symbols;
pub use profile::SCALA_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod predicates_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct ScalaPlugin;

impl LanguagePlugin for ScalaPlugin {
    fn id(&self) -> &str {
        "scala"
    }

    fn language_ids(&self) -> &[&str] {
        &["scala"]
    }

    fn extensions(&self) -> &[&str] {
        &[".scala", ".sc"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_scala::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::SCALA_SCOPE_KINDS
    }

    fn callback_lexical_adapter(
        &self,
    ) -> Option<&'static crate::indexer::callback_lexical::CallbackLexicalAdapter> {
        Some(&callback_lexical::ADAPTER)
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_definition",
            "object_definition",
            "trait_definition",
            "enum_definition",
            "full_enum_case",
            "simple_enum_case",
            "function_definition",
            "function_declaration",
            "val_definition",
            "var_definition",
            "val_declaration",
            "var_declaration",
            "type_definition",
            "given_definition",
            "package_clause",
            "package_object",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "instance_expression",
            "import_declaration",
            "export_declaration",
            "type_identifier",
            // type_arguments is intentionally excluded: generic type params like
            // `class Foo[A <: Bar, B]` produce multiple refs per node, breaking
            // the 1:1 node→ref coverage assumption (budget system only credits 1).
            // extends_clause is similarly excluded: `class Foo extends Bar with Baz`
            // produces both Inherits(Bar) and Implements(Baz) from one CST node.
            "infix_expression",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn signature_type_application(&self, text: &str) -> (String, Vec<String>) {
        crate::languages::bracket_type_application(text)
    }

    fn signature_type_head<'a>(&self, text: &'a str) -> &'a str {
        crate::languages::bracket_type_head(text)
    }

    fn type_text_policy(&self) -> crate::languages::TypeTextPolicy {
        crate::languages::TypeTextPolicy {
            fat_arrow_function: true,
            bare_arrow_parameter: true,
            parenthesized_tuple: true,
            bracket_application: true,
            ..crate::languages::TypeTextPolicy::OPAQUE
        }
    }

    fn signature_return_type(&self, signature: &str) -> Option<String> {
        crate::languages::colon_return_type(signature)
    }

    fn signature_parameter_types(&self, signature: &str) -> Option<Vec<String>> {
        crate::languages::colon_parameter_types(signature)
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::SCALA_PROFILE)
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::SCALA_FLOW_CONFIG)
    }

    fn flow_cfg_node_kinds(&self) -> Option<&'static crate::indexer::flow_cfg::CfgNodeKinds> {
        Some(&flow::SCALA_CFG_KINDS)
    }

    fn flow_return_query(&self) -> Option<&'static str> {
        Some(flow::SCALA_RETURN_QUERY)
    }

    fn flow_destructure_shape(
        &self,
        binding: tree_sitter::Node,
    ) -> crate::indexer::flow_assignments::DestructureShape {
        flow::destructure_shape(binding)
    }

    fn requires_correlated_flow_rebuild(&self) -> bool {
        true
    }

    fn augment_flow(
        &self,
        root: tree_sitter::Node,
        source: &[u8],
        symbols: &[crate::types::ExtractedSymbol],
        refs: &[crate::types::ExtractedRef],
        meta: &mut crate::types::FlowMeta,
    ) {
        flow::bind_match_tuple_cases(&root, source, symbols, refs, meta);
    }
}
