// =============================================================================
// r_lang/predicates.rs — R builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test" | "class"),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}


/// Common R packages that are never defined in the project but may appear as
/// the `module` qualifier in `pkg::fn` namespace-operator references.
pub(super) fn is_r_package(name: &str) -> bool {
    matches!(
        name,
        // tidyverse core
        "dplyr"
            | "ggplot2"
            | "tidyr"
            | "purrr"
            | "stringr"
            | "lubridate"
            | "forcats"
            | "tibble"
            | "readr"
            | "tidyselect"
            | "tidyverse"
            // import / export
            | "haven"
            | "readxl"
            | "writexl"
            | "jsonlite"
            | "httr"
            | "httr2"
            | "curl"
            | "xml2"
            | "rvest"
            // reporting / docs
            | "shiny"
            | "knitr"
            | "rmarkdown"
            | "htmltools"
            | "htmlwidgets"
            // dev tooling
            | "testthat"
            | "devtools"
            | "usethis"
            | "roxygen2"
            | "pkgload"
            | "pkgdown"
            | "covr"
            // data structures / utilities
            | "data.table"
            | "magrittr"
            | "rlang"
            | "vctrs"
            | "glue"
            | "fs"
            | "cli"
            | "crayon"
            | "withr"
            | "lifecycle"
            // stats / modelling
            | "broom"
            | "modelr"
            | "rsample"
            | "parsnip"
            | "recipes"
            | "workflows"
            | "yardstick"
            | "tune"
    )
}
