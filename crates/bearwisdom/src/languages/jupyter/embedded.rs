//! Jupyter embedded-region detection.
//!
//! One region per code cell. Cell bodies pass through
//! `magic::strip_magics` so Python/R parsers don't choke on
//! `!pip install` or `%timeit` lines. `line_offset` is taken from
//! the cell scanner so sub-extracted symbols land on the real line
//! of the `.ipynb` file.

use super::cell_scanner::{self, CellKind};
use super::magic;
use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let Some(nb) = cell_scanner::parse_notebook(source) else {
        return Vec::new();
    };
    let language_id = map_kernel_language(&nb.kernel_language);
    let mut regions = Vec::with_capacity(nb.cells.len());
    for cell in nb.cells {
        if cell.cell_type != CellKind::Code {
            continue;
        }
        let Some(lang_id) = language_id.as_deref() else {
            continue;
        };
        let cleaned = magic::strip_magics(&cell.body);
        regions.push(EmbeddedRegion {
            language_id: lang_id.to_string(),
            text: cleaned,
            line_offset: cell.body_line_offset,
            col_offset: 0,
            origin: EmbeddedOrigin::NotebookCell,
            holes: Vec::new(),
            strip_scope_prefix: None,
        });
    }
    regions
}

fn map_kernel_language(kernel: &str) -> Option<String> {
    let k = kernel.to_ascii_lowercase();
    let mapped = match k.as_str() {
        "python" | "python3" | "python2" => "python",
        "r" => "r",
        "julia" => return None, // Julia extractor not yet plumbed
        "javascript" | "js" | "node" => "javascript",
        "typescript" | "ts" => "typescript",
        "scala" => "scala",
        "rust" => "rust",
        "ruby" => "ruby",
        "bash" | "sh" => "bash",
        "powershell" | "pwsh" => "powershell",
        "csharp" | "c#" => "csharp",
        "fsharp" | "f#" => "fsharp",
        _ => return None,
    };
    Some(mapped.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_notebook_emits_python_region_per_code_cell() {
        let src = r##"{
 "cells": [
  {"cell_type": "code", "source": "x = 1\n", "metadata": {}},
  {"cell_type": "markdown", "source": "# Title\n", "metadata": {}},
  {"cell_type": "code", "source": "y = 2\n", "metadata": {}}
 ],
 "metadata": {"kernelspec": {"language": "python"}}
}"##;
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 2);
        assert!(regions.iter().all(|r| r.language_id == "python"));
        assert!(regions.iter().all(|r| r.origin == EmbeddedOrigin::NotebookCell));
    }

    #[test]
    fn magics_are_stripped_from_emitted_cell_text() {
        let src = r##"{
 "cells": [
  {"cell_type": "code", "source": "!pip install numpy\nimport numpy\n", "metadata": {}}
 ],
 "metadata": {"kernelspec": {"language": "python"}}
}"##;
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(!regions[0].text.contains("!pip"));
        assert!(regions[0].text.contains("import numpy"));
    }

    #[test]
    fn r_kernel_emits_r_regions() {
        let src = r##"{
 "cells": [
  {"cell_type": "code", "source": "library(dplyr)\n", "metadata": {}}
 ],
 "metadata": {"kernelspec": {"language": "R"}}
}"##;
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "r");
    }

    #[test]
    fn unknown_kernel_yields_no_regions() {
        let src = r##"{
 "cells": [{"cell_type": "code", "source": "x", "metadata": {}}],
 "metadata": {"kernelspec": {"language": "julia"}}
}"##;
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn malformed_notebook_yields_no_regions() {
        assert!(detect_regions("garbage").is_empty());
    }
}
