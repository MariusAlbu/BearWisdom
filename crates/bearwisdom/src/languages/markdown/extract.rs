//! Markdown host-level extraction.
//!
//! All Markdown host logic (file-stem symbol, ATX headings, link refs,
//! fence anchor symbols) lives in `host_scan.rs` so MDX can reuse it
//! without duplication. This module is a thin wrapper.

use super::host_scan;
use crate::types::ExtractionResult;

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let scan = host_scan::scan(source, file_path);
    ExtractionResult {
        symbols: scan.symbols,
        refs: scan.refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: false,
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EdgeKind, SymbolKind};

    #[test]
    fn extracts_atx_headings() {
        let src = "# Top\n\n## Sub\n\n### Deeper ###\n";
        let r = extract(src, "README.md");
        let h: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Field)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(h, vec!["Top", "Sub", "Deeper"]);
    }

    #[test]
    fn emits_file_host_symbol() {
        let src = "plain\n";
        let r = extract(src, "docs/overview.md");
        assert_eq!(r.symbols[0].name, "overview");
        assert_eq!(r.symbols[0].kind, SymbolKind::Class);
    }

    #[test]
    fn emits_fence_anchors() {
        let src = "```ts\nlet x = 1;\n```\n\n```python\nprint('x')\n```\n";
        let r = extract(src, "README.md");
        let anchors: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class && s.scope_path.is_some())
            .map(|s| s.name.as_str())
            .collect();
        assert!(anchors.contains(&"typescript#0"));
        assert!(anchors.contains(&"python#1"));
    }

    #[test]
    fn unknown_info_string_still_anchored_as_text() {
        let src = "```mermaid\ngraph\n```\n";
        let r = extract(src, "README.md");
        assert!(r.symbols.iter().any(|s| s.name == "text#0"));
    }

    #[test]
    fn relative_link_becomes_imports_ref() {
        let src = "See [overview](./architecture/overview.md) for details.\n";
        let r = extract(src, "README.md");
        let ref_targets: Vec<&str> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
        assert_eq!(ref_targets, vec!["architecture/overview"]);
        assert_eq!(r.refs[0].kind, EdgeKind::Imports);
    }

    #[test]
    fn external_link_ignored() {
        let src = "[site](https://example.com/foo) [mail](mailto:a@b.c)\n";
        let r = extract(src, "README.md");
        assert!(r.refs.is_empty());
    }

    #[test]
    fn anchor_only_link_ignored() {
        let src = "See [intro](#intro).\n";
        let r = extract(src, "README.md");
        assert!(r.refs.is_empty());
    }

    #[test]
    fn image_link_becomes_ref() {
        let src = "![alt](./images/logo.png)\n";
        let r = extract(src, "README.md");
        assert_eq!(r.refs.len(), 1);
        assert_eq!(r.refs[0].target_name, "images/logo");
    }
}
