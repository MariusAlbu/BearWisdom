//! Protocol Buffers language plugin.

pub mod connectors;
pub mod keywords;
pub mod extract;
pub(crate) mod hooks;
pub(crate) mod profile;

pub use hooks::PROTO_HOOKS;
pub use profile::PROTO_PROFILE;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct ProtoPlugin;

impl LanguagePlugin for ProtoPlugin {
    fn id(&self) -> &str {
        "proto"
    }

    fn language_ids(&self) -> &[&str] {
        &["proto"]
    }

    fn extensions(&self) -> &[&str] {
        &[".proto"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_proto::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source, tree_sitter_proto::LANGUAGE.into())
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "message",
            "service",
            "rpc",
            "enum",
            "enum_field",
            "field",
            "map_field",
            "package",
            "import",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "message_or_enum_type",
            "import",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        // Proto primitive types — these should not produce TypeRef edges
        &[
            "double", "float", "int32", "int64", "uint32", "uint64",
            "sint32", "sint64", "fixed32", "fixed64", "sfixed32", "sfixed64",
            "bool", "string", "bytes",
        ]
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::PROTO_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&hooks::PROTO_HOOKS)
    }
}
