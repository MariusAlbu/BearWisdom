//! Razor host-level extraction.
//!
//! Razor has no tree-sitter grammar in this workspace — every Razor
//! construct (`@{...}`, `@code{...}`, `@functions{...}`, `@(expr)`,
//! `@model`, `@inject`, `@using`, `@inherits`, `@implements`,
//! `@namespace`, `@if`/`@foreach`/`@while`/`@switch`/`@for`, and
//! `<script>` blocks) is handled by the embedded-region pipeline in
//! `embedded::detect_regions`.
//!
//! The host extractor itself emits:
//!   * A file-stem `Class` symbol so the host file is navigable and
//!     the script-ref `Imports` edges below have a valid source.
//!   * One `Imports` ref per `<script src="…">` tag, so the indexer's
//!     demand-driven script-tag stage can resolve the referenced vendor
//!     JS files under `wwwroot/lib/*` (which the standard walker
//!     excludes) and parse them with `origin='external'`.

use crate::languages::common::extract_script_refs;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let file_name = file_stem(file_path);
    let mut symbols = Vec::with_capacity(1);
    symbols.push(ExtractedSymbol {
        name: file_name.clone(),
        qualified_name: file_name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
    let host_index = 0usize;

    let script_refs = extract_script_refs(source);
    let mut refs = Vec::with_capacity(script_refs.len());
    for sr in script_refs {
        refs.push(ExtractedRef {
            source_symbol_index: host_index,
            target_name: sr.url.clone(),
            kind: EdgeKind::Imports,
            line: sr.line,
            module: Some(sr.url),
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
});
    }

    ExtractionResult {
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: false,
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
    }
}

fn file_stem(file_path: &str) -> String {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    stem.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_file_host_symbol() {
        let r = extract("<p>hi</p>", "Views/Home/Index.cshtml");
        assert_eq!(r.symbols.len(), 1);
        assert_eq!(r.symbols[0].name, "Index");
        assert_eq!(r.symbols[0].kind, SymbolKind::Class);
    }

    #[test]
    fn script_src_emits_imports_ref() {
        let src = r#"@{ Layout = "_Layout"; }
<script simpl-append-version="true" src="~/lib/jquery/jquery.js"></script>
<script src="~/lib/angular/angular.js"></script>"#;
        let r = extract(src, "Views/Shared/_Layout.cshtml");
        let targets: Vec<&str> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
        assert_eq!(
            targets,
            vec!["~/lib/jquery/jquery.js", "~/lib/angular/angular.js"]
        );
        assert!(r.refs.iter().all(|r| r.kind == EdgeKind::Imports));
    }

    #[test]
    fn cdn_script_ignored() {
        let src = r#"<script src="https://cdn.jsdelivr.net/npm/vue"></script>"#;
        let r = extract(src, "Views/Home/Index.cshtml");
        assert!(r.refs.is_empty());
    }
}
