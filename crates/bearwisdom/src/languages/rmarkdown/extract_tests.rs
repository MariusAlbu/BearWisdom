use super::extract;
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

#[test]
fn qmd_refs_source_symbol_index_in_bounds() {
    // Every ref emitted by the host-level Quarto extractor must have
    // source_symbol_index < symbols.len().
    let src = "# Title\n\n[link](other.qmd)\n\n```{python}\npass\n```\n";
    let r = extract(src, "doc.qmd");
    for rf in &r.refs {
        assert!(
            rf.source_symbol_index < r.symbols.len(),
            "REF-001: source_symbol_index {} out of bounds (symbols.len() = {}), ref target = {:?}",
            rf.source_symbol_index,
            r.symbols.len(),
            rf.target_name,
        );
    }
}
