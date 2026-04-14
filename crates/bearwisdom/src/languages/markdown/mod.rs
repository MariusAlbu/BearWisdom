//! Markdown (`.md`, `.markdown`, `.mdown`, `.mkd`, `.mkdn`) language
//! plugin — host extractor + embedded-region dispatcher for fenced
//! code blocks and frontmatter.
//!
//! The host extractor emits:
//!
//!   * A file-level `Class` symbol named after the Markdown file stem.
//!   * Heading symbols (ATX `# ... ######`) as `Field` kind.
//!   * Fence anchor symbols (one per fenced block) as `Class` kind so
//!     users can query "all TypeScript fenced examples in docs/".
//!   * `Imports` refs for relative links / images — the target is the
//!     linked file stem, which matches against other indexed files'
//!     host symbols (other Markdown, Astro, MDX).
//!
//! Embedded regions are produced by `embedded.rs`:
//!
//!   * Fenced blocks whose info-string normalizes to a known language
//!     id become `MarkdownFence` regions. Symbols spliced in from
//!     these regions flip `ParsedFile::symbol_from_snippet[i]` to
//!     true, which propagates to `unresolved_refs.from_snippet = 1`
//!     and excludes the ref from aggregate resolution stats.
//!
//!   * YAML (`---`), TOML (`+++`), or JSON (`{` at BOF) frontmatter
//!     becomes a `MarkdownFrontmatter` region. These are treated as
//!     config data, not snippets.

pub mod embedded;
pub mod extract;
pub mod fenced;
pub mod host_scan;
pub mod info_string;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct MarkdownPlugin;

impl LanguagePlugin for MarkdownPlugin {
    fn id(&self) -> &str {
        "markdown"
    }

    fn language_ids(&self) -> &[&str] {
        &["markdown", "md"]
    }

    fn extensions(&self) -> &[&str] {
        &[".md", ".markdown", ".mdown", ".mkd", ".mkdn"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        // The plugin uses a hand-rolled scanner (byte-level fence
        // detection) rather than tree-sitter-md, so the grammar is not
        // wired in — the extractor doesn't consume a tree-sitter tree.
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
