//! rust_lang language plugin.

mod calls;
pub(crate) mod connectors;
pub(crate) mod decorators;
mod embedded;
mod flow;
mod helpers;
mod patterns;
pub(crate) mod keywords;
mod symbols;
pub mod extract;

mod predicates;
mod chain;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::types::{EmbeddedRegion, ExtractionResult};
use crate::parser::scope_tree::ScopeKind;

pub use resolve::RustResolver;

pub struct RustLangPlugin;

impl LanguagePlugin for RustLangPlugin {
    fn id(&self) -> &str { "rust_lang" }

    fn language_ids(&self) -> &[&str] { &["rust"] }

    fn extensions(&self) -> &[&str] { &[".rs"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_rust::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn extract_connection_points(
        &self,
        source: &str,
        file_path: &str,
        _lang_id: &str,
    ) -> Vec<crate::types::ConnectionPoint> {
        connectors::extract_rust_connection_points(source, file_path)
    }

    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "struct_item",
            "enum_item",
            "enum_variant",
            "trait_item",
            "impl_item",
            "function_item",
            "function_signature_item",
            "const_item",
            "static_item",
            "type_item",
            "associated_type",
            "mod_item",
            "field_declaration",
            "union_item",
            "macro_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "macro_invocation",
            "struct_expression",
            // use_declaration is intentionally excluded: grouped multi-line imports
            // (use std::{io, fs};) emit refs at inner item lines, not the declaration
            // line, breaking the 1:1 node→ref coverage assumption.
            "impl_item",
            // type_cast_expression is excluded: casts to Rust primitives (x as u64)
            // correctly produce no ref (builtins are filtered), so most occurrences
            // don't generate refs — this is correct behavior, not a gap.
            "type_arguments",
            "attribute_item",
            "trait_bounds",
            "scoped_type_identifier",
            "type_identifier",
            "dynamic_type",
            "abstract_type",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::RustResolver))
    }

    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> {
        vec![
            Box::new(connectors::TauriIpcConnector),
            Box::new(connectors::RustRestConnector),
            Box::new(connectors::RustGrpcConnector),
            Box::new(connectors::RustMqConnector),
        ]
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::RUST_FLOW_CONFIG)
    }
}