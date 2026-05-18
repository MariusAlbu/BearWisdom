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
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
    }
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod tests;
