//! rust_lang language plugin.

mod calls;
mod calls_args;
mod calls_imports;
mod calls_macros;
pub(crate) mod decorators;
mod derives;
mod embedded;
pub(crate) mod flow;
mod flow_detectors;
mod helpers;
mod patterns;
pub(crate) mod keywords;
mod symbols;
pub mod extract;

mod predicates;
pub(crate) mod hooks;
pub(crate) mod profile;
pub use hooks::RUST_HOOKS;
pub use profile::RUST_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

#[cfg(test)]
#[path = "derives_tests.rs"]
mod derives_tests;

use crate::languages::{LanguagePlugin, Synthesized};
use crate::types::{EmbeddedRegion, ExtractedRef, ExtractedSymbol, ExtractionResult};
use crate::parser::scope_tree::ScopeKind;

pub struct RustLangPlugin;

impl LanguagePlugin for RustLangPlugin {
    // The module is `rust_lang` (the plain `rust` name would shadow the
    // language tag downstream code uses); the plugin's public id and
    // language_ids both return `"rust"` so the registry's `by_lang_id` and
    // extension-routed `language_id_for_extension` paths agree. They were
    // out of sync before — `id()` returned `"rust_lang"` while
    // `language_ids()` returned `["rust"]`. The registry keys `by_lang_id`
    // only by `language_ids`, but `language_by_extension` falls back to
    // `id()`. So files routed through extension lookup landed with
    // language `"rust_lang"`, then `registry.get("rust_lang")` missed and
    // returned the generic fallback plugin — emitting zero Rust symbols
    // for every cargo dep file demand-pulled by `expand.rs`.
    fn id(&self) -> &str { "rust" }

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

    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(source)
    }

    fn synthesize_symbols(
        &self,
        source: &str,
        symbols: &[ExtractedSymbol],
        refs: &[ExtractedRef],
    ) -> Synthesized {
        derives::synthesize_derive_members(source, symbols, refs)
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

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::RUST_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&hooks::RUST_HOOKS)
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::RUST_FLOW_CONFIG)
    }
}