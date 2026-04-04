//! HCL / Terraform language plugin.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct HclPlugin;

impl LanguagePlugin for HclPlugin {
    fn id(&self) -> &str {
        "hcl"
    }

    fn language_ids(&self) -> &[&str] {
        &["hcl", "terraform"]
    }

    fn extensions(&self) -> &[&str] {
        &[".hcl", ".tf", ".tfvars"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_hcl::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        let _ = source;
        ExtractionResult::empty()
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "block",
            "attribute",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "variable_expr",
            "get_attr",
            "function_call",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }
}
