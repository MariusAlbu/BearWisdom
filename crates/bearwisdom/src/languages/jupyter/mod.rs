//! Jupyter Notebook (`.ipynb`) language plugin.
//!
//! A Jupyter notebook is a JSON document with a `cells` array. Each
//! cell has a `cell_type` (`code` or `markdown`) and a `source` that
//! is either a string or a list of strings (concatenated to form the
//! cell body). The kernel language is declared in
//! `metadata.kernelspec.language` and defaults to Python.
//!
//! This plugin:
//!
//!   * Emits a file-stem host symbol plus one cell-anchor symbol per
//!     code cell so `file_symbols` on an `.ipynb` shows a structured
//!     outline.
//!   * Dispatches each code cell's body to the kernel's language
//!     plugin via `embedded_regions`. Magic lines (`!shell`, `%magic`,
//!     `%%cellmagic`) are blanked to preserve line numbers while
//!     keeping the cell body syntactically valid for the parser.
//!   * Honors the `source` field's byte position in the original JSON
//!     text so sub-extracted symbol line numbers land on the real
//!     line of the `.ipynb` file.

pub mod cell_scanner;
pub mod embedded;
pub mod extract;
pub mod magic;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct JupyterPlugin;

impl LanguagePlugin for JupyterPlugin {
    fn id(&self) -> &str {
        "jupyter"
    }
    fn language_ids(&self) -> &[&str] {
        &["jupyter", "ipynb"]
    }
    fn extensions(&self) -> &[&str] {
        &[".ipynb"]
    }
    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        None
    }
    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }
    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source, file_path)
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
        &[]
    }
    fn ref_node_kinds(&self) -> &[&str] {
        &[]
    }
}
