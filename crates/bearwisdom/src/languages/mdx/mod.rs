//! MDX (`.mdx`) language plugin — Markdown-with-JSX host.
//!
//! MDX files are Markdown prose that may contain:
//!
//!   * ES `import` / `export` statements at the top level (collected
//!     into a single TypeScript `ScriptBlock` region so the TS
//!     extractor emits symbols + import refs);
//!   * JSX component elements inline (`<Button variant="x" />`,
//!     `<Foo.Bar>` — emitted as `Calls` refs against the host file
//!     symbol, resolvable to components imported at the top);
//!   * Fenced code blocks with info-strings (same dispatch as
//!     Markdown — see `markdown::embedded`);
//!   * YAML / TOML frontmatter (same as Markdown).
//!
//! Shared host logic (file-stem symbol, ATX headings, link refs,
//! fence anchors) lives in `markdown::host_scan` so MDX and Markdown
//! don't duplicate it.

pub mod embedded;
pub mod extract;
pub mod resolve;

use std::sync::Arc;

use crate::indexer::resolve::engine::LanguageResolver;
use crate::languages::LanguagePlugin;
use resolve::MdxResolver;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct MdxPlugin;

impl LanguagePlugin for MdxPlugin {
    fn id(&self) -> &str {
        "mdx"
    }

    fn language_ids(&self) -> &[&str] {
        &["mdx"]
    }

    fn extensions(&self) -> &[&str] {
        &[".mdx"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        // MDX uses a hand-rolled scanner — no tree-sitter tree consumed.
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

    fn resolver(&self) -> Option<Arc<dyn LanguageResolver>> {
        // MDX needs both halves of resolution: Markdown-style relative
        // link Imports (path probing) AND TS-import-aware JSX Calls
        // (cross-reference component refs against the file's spliced TS
        // import bindings). MdxResolver dispatches by ref kind — the
        // same pattern Vue/Svelte use to bridge template tags onto the
        // `<script>` block's TypeScript imports.
        Some(Arc::new(MdxResolver))
    }
}
