//! Host-level extraction for RMarkdown and Quarto files. Reuses
//! `markdown::host_scan` for headings, fence anchors, link refs, and
//! the file-stem symbol.

use super::super::markdown::host_scan;
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
    use crate::types::SymbolKind;

    #[test]
    fn rmd_headings_extracted() {
        let src = "---\ntitle: Rpt\n---\n\n# Top\n\n## Analysis\n";
        let r = extract(src, "report.Rmd");
        assert!(r.symbols.iter().any(|s| s.name == "report"));
        let fields: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Field)
            .map(|s| s.name.as_str())
            .collect();
        assert!(fields.contains(&"Top"));
        assert!(fields.contains(&"Analysis"));
    }

    #[test]
    fn qmd_chunk_becomes_fence_anchor() {
        let src = "# Title\n\n```{python}\nimport pandas as pd\n```\n";
        let r = extract(src, "doc.qmd");
        let anchor_names: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class && s.scope_path.is_some())
            .map(|s| s.name.as_str())
            .collect();
        assert!(anchor_names.contains(&"python#0"));
    }
}
