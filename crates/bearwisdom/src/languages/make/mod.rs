//! Make / Makefile language plugin.
//!
//! Grammar status: tree-sitter-make is not in Cargo.toml yet.
//! The `grammar()` method returns `None`, falling back to the generic extractor,
//! until the crate is added. The extraction logic in `extract.rs` is ready for
//! when the grammar is wired in.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct MakePlugin;

impl LanguagePlugin for MakePlugin {
    fn id(&self) -> &str {
        "make"
    }

    fn language_ids(&self) -> &[&str] {
        &["make"]
    }

    /// Extensions for Make files. `Makefile` (no dot) is detected by
    /// bearwisdom-profile via filename matching.
    fn extensions(&self) -> &[&str] {
        &["Makefile", ".mk"]
    }

    /// Returns `None` until tree-sitter-make is added to Cargo.toml.
    /// Falls back to the generic extractor in the meantime.
    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        None
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        // Grammar not available yet — return empty until grammar is wired in.
        // extract::extract(source) is ready for when it is.
        let _ = source;
        ExtractionResult::empty()
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "rule",
            "variable_assignment",
            "define_directive",
            "shell_assignment",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "include_directive",
            "function_call",
            "shell_function",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }
}
