//! TypeScript / TSX / JavaScript / JSX language plugin.
//!
//! Handles extraction for all four language IDs. The TypeScript and JavaScript
//! grammars are separate tree-sitter crates but share most extraction logic.
//! TSX and JSX use their respective grammars for JSX support.

// Extraction sub-modules
mod alias_classify;
mod angular_module_reachables;
pub(crate) mod alias_intrinsics;
mod alias_type_text;
mod alias_union;
pub(crate) mod ambient_modules;
mod annotation_members;
mod annotation_named_type;
mod calls;
pub(crate) mod component_tags;
pub mod connectors;
mod connectors_graphql;
mod connectors_nestjs;
mod connectors_nextjs;
mod connectors_react;
pub(crate) mod decorators;
mod embedded;
pub(crate) mod external_virtual_path;
pub(crate) mod flow;
mod helpers;
mod imports;
pub(crate) mod keywords;
mod module_augmentations;
pub(crate) mod module_policy;
mod narrowing;
mod params;
mod qualify_members;
mod symbols;
mod symbols_casts;
mod symbols_fields;
mod symbols_variables;
mod types;

mod expressions;
pub mod extract;
mod reexports;
pub(crate) mod selectors;
mod signature;
mod type_scan;

// Resolution sub-modules
pub(crate) mod predicates;
pub mod profile;
pub use profile::TYPESCRIPT_PROFILE;

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
#[path = "module_augmentations_tests.rs"]
mod module_augmentations_tests;

#[cfg(test)]
#[path = "type_text_tests.rs"]
mod type_text_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

/// TypeScript language plugin — handles "typescript", "tsx", "javascript", "jsx".
pub struct TypeScriptPlugin;

impl LanguagePlugin for TypeScriptPlugin {
    fn id(&self) -> &str {
        "typescript"
    }

    fn language_ids(&self) -> &[&str] {
        &["typescript", "tsx"]
    }

    fn extensions(&self) -> &[&str] {
        &[".ts", ".tsx", ".mts", ".cts"]
    }

    /// `.tsx` uses the TSX grammar, so the language id must be "tsx" (not the
    /// plugin's primary "typescript") for `grammar(lang_id)` to pick the
    /// right parser. Other extensions route to the TypeScript grammar.
    fn language_id_for_extension(&self, ext: &str) -> Option<&str> {
        match ext.to_ascii_lowercase().as_str() {
            ".tsx" => Some("tsx"),
            ".ts" | ".mts" | ".cts" => Some("typescript"),
            _ => None,
        }
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        Some(match lang_id {
            "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
            _ => return None,
        })
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::TS_SCOPE_KINDS
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let is_tsx = file_path.ends_with(".tsx") || lang_id == "tsx";
        let mut result = extract::extract(source, is_tsx);
        crate::languages::common::append_ember_helper_default_export(
            file_path,
            source,
            &mut result,
        );
        crate::languages::common::append_handlebars_register_helper_globals(source, &mut result);
        result
    }

    fn external_virtual_path(
        &self,
        language: &str,
        normalized_absolute_path: &str,
    ) -> Option<String> {
        matches!(language, "typescript" | "tsx")
            .then(|| external_virtual_path::for_pulled(normalized_absolute_path))
            .flatten()
    }

    fn signature_type_application(&self, text: &str) -> (String, Vec<String>) {
        crate::languages::angle_type_application(text)
    }

    fn signature_type_head<'a>(&self, text: &'a str) -> &'a str {
        crate::languages::angle_type_head(text)
    }

    fn type_text_policy(&self) -> crate::languages::TypeTextPolicy {
        crate::languages::TypeTextPolicy {
            fat_arrow_function: true,
            readonly_modifier: true,
            array_suffix: true,
            union_intersection: true,
            bracket_tuple: true,
            angle_application: true,
            ..crate::languages::TypeTextPolicy::OPAQUE
        }
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
        matches!(head, "Array" | "ReadonlyArray")
    }
    fn signature_return_type(&self, signature: &str) -> Option<String> {
        signature::return_type(signature)
    }

    fn signature_parameter_types(&self, signature: &str) -> Option<Vec<String>> {
        signature::parameter_types(signature)
    }

    fn signature_declared_type(&self, signature: &str) -> Option<String> {
        signature::declared_type(signature)
    }

    fn signature_object_type_members(&self, signature: &str) -> Vec<(String, String)> {
        signature::object_type_members(signature)
    }

    fn signature_conditional_branches(&self, signature: &str) -> Option<(String, String)> {
        signature::conditional_branches(signature)
    }

    fn signature_generic_params(
        &self,
        signature: &str,
        name: &str,
    ) -> Vec<(String, Option<String>, Option<String>)> {
        signature::generic_params(signature, name)
    }
    fn signature_is_inline_object_result(&self, result: &str) -> bool {
        signature::is_inline_object_result(result)
    }

    fn signature_is_tuple_result(&self, result: &str) -> bool {
        signature::is_tuple_result(result)
    }

    fn signature_has_callable_return_extraction(&self, result: &str) -> bool {
        signature::has_callable_return_extraction(result)
    }
    fn signature_field_type(&self, signature: &str) -> Option<String> {
        let valid = !matches!(signature, "true" | "false" | "null" | "undefined")
            && !signature.is_empty()
            && signature
                .chars()
                .all(|ch| ch == '_' || ch == '$' || ch.is_ascii_alphanumeric())
            && signature
                .chars()
                .next()
                .is_some_and(|ch| ch == '_' || ch == '$' || ch.is_ascii_alphabetic());
        valid.then(|| signature.to_string())
    }

    fn alias_intrinsic(
        &self,
        head: &str,
    ) -> Option<crate::type_checker::profile::chain_specs::AliasIntrinsic> {
        alias_intrinsics::member_resolution_intrinsic(head)
    }

    fn flat_callable_return_operand<'a>(&self, head: &'a str) -> Option<&'a str> {
        alias_intrinsics::flat_callable_return_operand(head)
    }

    fn callable_return_operand<'a>(&self, head: &'a str) -> Option<&'a str> {
        alias_intrinsics::callable_return_operand(head)
    }

    fn is_callable_return_extractor(
        &self,
        arena: &crate::type_checker::core::types::TypeArena,
        target: &crate::types::AliasTargetIds,
    ) -> bool {
        alias_intrinsics::is_callable_return_extractor(arena, target)
    }

    fn external_module_augmentations(
        &self,
        source: &str,
        virtual_path: &str,
    ) -> Vec<crate::languages::ModuleAugmentation> {
        module_augmentations::collect(source, virtual_path)
    }

    fn external_reexport_target_qname(
        &self,
        virtual_path: &str,
        target_name: &str,
    ) -> Option<String> {
        module_augmentations::external_reexport_target_qname(virtual_path, target_name)
    }

    fn extract_with_demand(
        &self,
        source: &str,
        file_path: &str,
        lang_id: &str,
        demand: Option<&std::collections::HashSet<String>>,
    ) -> ExtractionResult {
        let is_tsx = file_path.ends_with(".tsx") || lang_id == "tsx";
        extract::extract_with_demand(source, is_tsx, demand)
    }

    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(source, lang_id)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "abstract_class_declaration",
            "interface_declaration",
            "function_declaration",
            "generator_function_declaration",
            "method_definition",
            "abstract_method_signature",
            "method_signature",
            "public_field_definition",
            "property_signature",
            "field_definition",
            "type_alias_declaration",
            "enum_declaration",
            "lexical_declaration",
            "variable_declaration",
            "internal_module",
            "construct_signature",
            "call_signature",
            "index_signature",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "new_expression",
            "import_statement",
            // jsx_self_closing_element and jsx_opening_element are intentionally excluded:
            // we only emit refs for PascalCase component tags (~23% of occurrences),
            // not HTML intrinsics (div, span, etc.), so the 1:1 node→ref assumption breaks.
            "extends_clause",
            "implements_clause",
            "type_annotation",
            "type_identifier",
            "as_expression",
            "satisfies_expression",
            "tagged_template_expression",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&TYPESCRIPT_PROFILE)
    }

    fn component_tag_head<'a>(&self, target: &'a str) -> Option<&'a str> {
        component_tags::component_tag_head(target)
    }

    fn is_component_file(&self, path: &str) -> bool {
        component_tags::is_component_file(path)
    }

    fn selector_binding_keys(&self, selector: &str) -> Vec<String> {
        selectors::selector_binding_keys(selector)
    }

    fn component_selectors(
        &self,
        source: &str,
        symbols: &[crate::types::ExtractedSymbol],
    ) -> Vec<(String, String)> {
        let mut selectors = selectors::extract_component_selectors(source, symbols);
        selectors.extend(selectors::extract_custom_element_defines(source, symbols));
        selectors
    }

    fn exact_module_entry_aliases(&self, declared_modules: &[String]) -> Vec<String> {
        declared_modules
            .iter()
            .filter(|name| !name.contains('*'))
            .cloned()
            .collect()
    }

    fn plugin_flow_emissions(
        &self,
        source: &str,
        _file_path: &str,
    ) -> Vec<(u32, crate::indexer::resolve::flow_emit::FlowEmission)> {
        connectors::extract_typescript_graphql(source)
    }

    fn discover_routes(
        &self,
        conn: &rusqlite::Connection,
        project_root: &std::path::Path,
        project_ctx: &crate::indexer::project_context::ProjectContext,
    ) -> u32 {
        connectors::discover_nestjs_routes(conn, project_root, project_ctx)
            + connectors::discover_nextjs_routes(conn, project_root, project_ctx)
    }

    fn post_index(
        &self,
        db: &crate::db::Database,
        project_root: &std::path::Path,
        _ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        connectors::run_react_patterns(db.conn(), project_root);
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::TS_FLOW_CONFIG)
    }

    fn flow_strategy_aliases(&self) -> &'static [&'static str] {
        &["ts"]
    }

    fn flow_cfg_node_kinds(&self) -> Option<&'static crate::indexer::flow_cfg::CfgNodeKinds> {
        Some(&flow::TS_CFG_KINDS)
    }

    fn flow_return_query(&self) -> Option<&'static str> {
        Some(flow::TS_RETURN_QUERY)
    }

    fn flow_return_object_members(
        &self,
        node: tree_sitter::Node,
        source: &[u8],
    ) -> Option<Vec<String>> {
        flow::return_object_members(node, source)
    }

    fn flow_destructure_shape(
        &self,
        binding: tree_sitter::Node,
    ) -> crate::indexer::flow_assignments::DestructureShape {
        flow::destructure_shape(binding)
    }

    fn flow_is_await_rhs(&self, node: tree_sitter::Node) -> bool {
        flow::is_await_rhs(node)
    }

    fn normalize_flow_guard_type(&self, raw: &str) -> Option<String> {
        flow::normalize_guard_type(raw)
    }

    fn flow_discriminant_early_exit_scope(&self, node: tree_sitter::Node) -> Option<(u32, u32)> {
        flow::discriminant_early_exit_scope(node)
    }

    fn lexical_syntax(&self) -> Option<&'static crate::indexer::lexical::LexicalSyntax> {
        Some(&flow::TS_LEXICAL_SYNTAX)
    }

    fn external_declaration_reachables(&self, file_path: &str, content: &str) -> Vec<String> {
        angular_module_reachables::reachables(file_path, content)
    }

    fn source_module_path_policy(
        &self,
        _specifier: &str,
    ) -> crate::type_checker::profile::language_profile::SourceModulePathPolicy {
        module_policy::SOURCE_MODULE_PATH_POLICY
    }
}
