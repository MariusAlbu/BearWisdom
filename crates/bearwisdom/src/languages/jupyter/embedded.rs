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

/// Map a notebook's declared kernel/language name to a registry language id.
///
/// Accepts the three forms a notebook can supply: a canonical language name
/// (`language_info.name` — "R", "python", "julia"), the legacy
/// `kernelspec.language` field, or the kernelspec id (`kernelspec.name` —
/// "ir", "python3", "ijavascript"). Returns `None` for kernels whose
/// extractor isn't plumbed yet (Julia).
fn map_kernel_language(kernel: &str) -> Option<String> {
    let k = kernel.to_ascii_lowercase();
    let mapped = match k.as_str() {
        // Canonical names.
        "python" | "python3" | "python2" => "python",
        "r" => "r",
        "javascript" | "js" | "node" => "javascript",
        "typescript" | "ts" => "typescript",
        "scala" => "scala",
        "rust" => "rust",
        "ruby" => "ruby",
        "bash" | "sh" => "bash",
        "powershell" | "pwsh" => "powershell",
        "csharp" | "c#" => "csharp",
        "fsharp" | "f#" => "fsharp",
        // Kernel ids — the IRkernel ships as "ir", IJulia as "julia",
        // ITypeScript as "tslab", etc. Map the common kernel ids to the
        // language they execute so `kernelspec.name` fallback works.
        "ir" => "r",
        "ijavascript" => "javascript",
        "tslab" => "typescript",
        "iruby" => "ruby",
        "iscala" | "scala-kernel" => "scala",
        "ipowershell" => "powershell",
        "julia" | "ijulia" => return None, // extractor not yet plumbed
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
        assert!(regions
            .iter()
            .all(|r| r.origin == EmbeddedOrigin::NotebookCell));
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
    fn r_notebook_with_irkernel_metadata_emits_r_regions() {
        // Real-world IRkernel notebook: kernelspec.name = "ir",
        // language_info.name = "R". Both paths should converge on "r".
        let src = r##"{
 "cells": [
  {"cell_type": "code", "source": "library(dplyr)\n", "metadata": {}}
 ],
 "metadata": {
   "kernelspec": {"name": "ir", "display_name": "R"},
   "language_info": {"name": "R"}
 }
}"##;
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "r");
    }

    #[test]
    fn r_notebook_with_only_kernelspec_name_ir_still_emits_r_regions() {
        let src = r##"{
 "cells": [
  {"cell_type": "code", "source": "library(dplyr)\n", "metadata": {}}
 ],
 "metadata": {"kernelspec": {"name": "ir"}}
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
