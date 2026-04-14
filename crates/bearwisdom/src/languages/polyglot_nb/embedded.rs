//! Polyglot Notebook embedded-region detection.
//!
//! One region per code cell, dispatched to the kernel's language
//! plugin via `kernel_to_language_id`. Non-code cells (`value`,
//! `mermaid`, markdown prose) are skipped — markdown cells could be
//! dispatched to the Markdown plugin, but their content usually
//! just contains prose + occasional fenced code, which doesn't
//! benefit from nested dispatch at this stage.

use super::cells;
use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    for cell in cells::parse_cells(source) {
        let Some(lang) = cells::kernel_to_language_id(&cell.kernel) else {
            continue;
        };
        // Skip markdown cells for now — same rationale as the MDX
        // plugin: we'd dispatch to the markdown plugin, which would
        // re-dispatch its fenced blocks. Notebook markdown is
        // prose-dominant, so the indirection adds noise with little
        // gain.
        if lang == "markdown" {
            continue;
        }
        regions.push(EmbeddedRegion {
            language_id: lang.to_string(),
            text: cell.body,
            line_offset: cell.body_line_offset,
            col_offset: 0,
            origin: EmbeddedOrigin::NotebookCell,
            holes: Vec::new(),
            strip_scope_prefix: None,
        });
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csharp_cell_becomes_region() {
        let src = "#!csharp\nvar x = 1;\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "csharp");
        assert_eq!(regions[0].origin, EmbeddedOrigin::NotebookCell);
        assert!(regions[0].text.contains("var x = 1;"));
    }

    #[test]
    fn multiple_kernels_produce_multiple_regions() {
        let src = "#!csharp\nvar a = 1;\n\n#!fsharp\nlet b = 2\n\n#!pwsh\n$c = 3\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 3);
        assert_eq!(regions[0].language_id, "csharp");
        assert_eq!(regions[1].language_id, "fsharp");
        assert_eq!(regions[2].language_id, "powershell");
    }

    #[test]
    fn non_code_kernels_skipped() {
        let src = "#!csharp\nvar x = 1;\n\n#!mermaid\ngraph TD\n\n#!value\nfoo\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "csharp");
    }

    #[test]
    fn markdown_cell_skipped() {
        let src = "#!markdown\n# Hello\n\n#!csharp\nvar x = 1;\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "csharp");
    }

    #[test]
    fn region_line_offset_is_first_body_line() {
        let src = "#!csharp\nvar x = 1;\n";
        let regions = detect_regions(src);
        assert_eq!(regions[0].line_offset, 1);
    }

    #[test]
    fn empty_source_no_regions() {
        assert!(detect_regions("").is_empty());
    }
}
