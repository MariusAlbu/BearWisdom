//! RMarkdown (`.Rmd`) and Quarto (`.qmd`) language plugin.
//!
//! Both formats share the knitr chunk convention: markdown prose
//! interleaved with fenced code blocks whose info-string is wrapped
//! in braces — `{r}`, `{python}`, `{bash}`, `{r chunk-name,
//! echo=FALSE}`, etc. Frontmatter (YAML `---` fence) is standard.
//!
//! Implementation reuses the Markdown plugin's scanners: `fenced`
//! parses the blocks, `info_string` normalizes braced chunk headers
//! to canonical language ids. Cell dispatch uses
//! `EmbeddedOrigin::NotebookCell` so the symbols count toward
//! project resolution stats — these are runnable source files, not
//! doc snippets.

pub mod embedded;
pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct RMarkdownPlugin;

impl LanguagePlugin for RMarkdownPlugin {
    fn id(&self) -> &str {
        "rmarkdown"
    }
    fn language_ids(&self) -> &[&str] {
        &["rmarkdown", "rmd"]
    }
    fn extensions(&self) -> &[&str] {
        &[".Rmd", ".rmd"]
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

pub struct QuartoPlugin;

impl LanguagePlugin for QuartoPlugin {
    fn id(&self) -> &str {
        "quarto"
    }
    fn language_ids(&self) -> &[&str] {
        &["quarto", "qmd"]
    }
    fn extensions(&self) -> &[&str] {
        &[".qmd"]
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
