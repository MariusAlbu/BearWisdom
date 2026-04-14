//! .NET Polyglot Notebook (`.dib`, `.dotnet-interactive`) language plugin.
//!
//! Polyglot notebooks are a Microsoft .NET Interactive format: plain
//! text file, cells delimited by `#!<lang>` markers at the start of a
//! line. Recognized cell kernels:
//!
//!   * `#!csharp`   → csharp
//!   * `#!fsharp`   → fsharp
//!   * `#!pwsh` / `#!powershell` → powershell
//!   * `#!javascript`            → javascript
//!   * `#!sql` / `#!kql`         → sql
//!   * `#!html`                  → html
//!   * `#!markdown`              → markdown
//!   * `#!mermaid` / `#!value`   → skipped (non-code)
//!
//! The host extractor emits a file-level symbol named after the
//! notebook stem plus one heading-like symbol per cell so `file_symbols`
//! on a `.dib` shows a cell outline. Embedded regions dispatch each
//! code cell to its language plugin with `origin = NotebookCell`.

pub mod cells;
pub mod extract;
pub mod embedded;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct PolyglotNbPlugin;

impl LanguagePlugin for PolyglotNbPlugin {
    fn id(&self) -> &str {
        "polyglot_nb"
    }

    fn language_ids(&self) -> &[&str] {
        &["polyglot_nb", "dib"]
    }

    fn extensions(&self) -> &[&str] {
        &[".dib", ".dotnet-interactive"]
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
