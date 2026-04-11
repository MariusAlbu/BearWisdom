use std::collections::HashSet;

/// Runtime globals always external for R.
///
/// R's C-level internal API names that appear in `.c`/`.cpp` files within R
/// packages (via R_RegisterCCallable, PROTECT, etc.). These are never defined
/// in R source and are not covered by primitives.rs.
pub(crate) const EXTERNALS: &[&str] = &[
    // C API — memory protection
    "PROTECT", "UNPROTECT", "UNPROTECT_PTR",
    // C API — type aliases / constructors
    "SEXP", "R_xlen_t", "SEXPREC",
    "ScalarInteger", "ScalarReal", "ScalarLogical", "ScalarString",
    "ScalarComplex", "ScalarRaw",
    "allocVector", "allocMatrix", "allocArray",
    "mkChar", "mkString",
    // C API — accessors
    "INTEGER", "REAL", "LOGICAL", "RAW", "COMPLEX", "STRING_ELT",
    "SET_STRING_ELT", "VECTOR_ELT", "SET_VECTOR_ELT",
    "LENGTH", "XLENGTH", "Rf_length",
    "TYPEOF", "NAMED",
    // C API — evaluation
    "eval", "Rf_eval", "R_tryEval",
    "CAR", "CDR", "CAAR", "CDAR", "CADR", "CDDR", "CADDR", "CDDDR",
    "CONS", "LCONS",
    // R_NilValue and friends
    "R_NilValue", "R_UnboundValue", "R_GlobalEnv", "R_BaseEnv",
    "R_EmptyEnv", "R_NaString", "R_BlankString",
    // C error / warning
    "Rf_error", "Rf_warning",
];

/// Dependency-gated framework globals for R.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    if deps.contains("testthat") {
        globals.extend(TESTTHAT_GLOBALS);
    }

    // Tidyverse globals are unconditional for R. BearWisdom has no parser
    // for R's DESCRIPTION manifest, so `deps` is always empty for R
    // projects — gating on `deps.contains("rlang")` / etc. never fires.
    // In practice every modern R package uses rlang / vctrs / cli / glue
    // / lifecycle (they're re-exported by tibble and base tidyverse),
    // and the collision risk with project-defined names is negligible
    // — the underscore-and-snake-case naming of tidyverse helpers rarely
    // matches user-written functions.
    globals.extend(RLANG_GLOBALS);
    globals.extend(VCTRS_TIBBLE_GLOBALS);
    globals.extend(DPLYR_TIDYR_GLOBALS);
    globals.extend(&["glue", "glue_collapse", "glue_sql", "glue_data"]);

    if deps.contains("usethis") {
        globals.extend(&[
            "use_package", "use_r", "use_test", "use_data",
            "use_vignette", "use_readme_md", "use_news_md",
            "proj_get", "proj_set",
        ]);
    }
    if deps.contains("devtools") {
        globals.extend(&[
            "document", "check", "install", "build", "load_all",
            "test", "run_examples",
        ]);
    }
    if deps.contains("shiny") {
        globals.extend(SHINY_GLOBALS);
    }

    globals
}

const TESTTHAT_GLOBALS: &[&str] = &[
    "test_that", "describe", "it", "context", "setup", "teardown",
    "expect_equal", "expect_identical", "expect_true", "expect_false",
    "expect_null", "expect_error", "expect_warning", "expect_message",
    "expect_condition", "expect_output", "expect_silent",
    "expect_invisible", "expect_visible",
    "expect_type", "expect_s3_class", "expect_s4_class",
    "expect_length", "expect_match", "expect_named",
    "expect_setequal", "expect_mapequal",
    "expect_gt", "expect_gte", "expect_lt", "expect_lte",
    "expect_snapshot", "expect_snapshot_output",
    "expect_snapshot_error", "expect_snapshot_value",
    "expect_no_error", "expect_no_warning",
    "expect_no_message", "expect_no_condition",
    "skip", "skip_if", "skip_if_not",
    "skip_on_cran", "skip_on_ci", "skip_on_os",
    "local_edition", "with_mocked_bindings",
    "test_path", "test_check", "test_file", "test_dir",
];

const RLANG_GLOBALS: &[&str] = &[
    "abort", "warn", "inform", "signal",
    "is_null", "is_na", "is_true", "is_false",
    "is_character", "is_double", "is_integer", "is_logical",
    "is_list", "is_vector", "is_function", "is_closure",
    "is_empty", "is_scalar_character", "is_scalar_double",
    "is_scalar_integer", "is_scalar_logical",
    "quo", "enquo", "quos", "enquos", "expr", "enexpr",
    "sym", "syms", "ensym", "ensyms",
    "eval_tidy", "eval_bare",
    "new_function", "new_environment",
    "env_get", "env_set", "env_has", "env_bind",
    "call2", "call_match",
    "as_label", "as_name",
    "dots_list", "list2", "modify_list",
    "set_names", "has_name",
    "local_options", "with_options",
    "try_fetch",
    "check_dots_empty0", "check_dots_empty", "check_dots_unnamed",
    "caller_env", "current_env", "global_env",
    "caller_fn", "caller_call",
    "frame_call", "trace_back",
    "arg_match", "arg_match0",
    "exec",
    "inject", "splice", "fn_fmls", "fn_fmls_names", "fn_fmls_syms",
    "rlang::abort", "rlang::warn", "rlang::inform",
];

/// vctrs + tibble helpers. vctrs provides R's typed vector runtime and is
/// re-exported by most tidyverse packages. tibble is the tidyverse's
/// replacement for data.frame.
const VCTRS_TIBBLE_GLOBALS: &[&str] = &[
    "vec_slice", "vec_ptype", "vec_ptype2", "vec_cast",
    "vec_c", "vec_rbind", "vec_cbind",
    "vec_size", "vec_size0", "vec_is", "vec_is_list",
    "vec_recycle", "vec_recycle_common",
    "vec_assert", "vec_assert2",
    "vec_default_cast", "vec_default_ptype2",
    "vec_init", "vec_unique", "vec_count", "vec_duplicate_any",
    "vec_match", "vec_in", "vec_group_id",
    "list_of", "list_sizes", "list_drop_empty",
    "data_frame", "new_data_frame",
    "tibble", "new_tibble", "as_tibble", "as_tibble_row", "as_tibble_col",
    "tribble", "is_tibble", "validate_tibble",
    "rowid_to_column",
    "vctrs::data_frame", "vctrs::new_data_frame",
    "tibble::tibble", "tibble::as_tibble",
];

/// dplyr + tidyr verbs and helpers. These appear as bare identifiers in
/// any tidyverse pipeline. Most are also re-exported by tidyverse meta-
/// package, so we activate the list whenever any tidyverse package is a
/// dependency.
const DPLYR_TIDYR_GLOBALS: &[&str] = &[
    // dplyr verbs
    "mutate", "transmute", "filter", "arrange", "select", "rename",
    "summarise", "summarize", "group_by", "ungroup", "groups",
    "distinct", "slice", "slice_head", "slice_tail", "slice_sample",
    "slice_min", "slice_max",
    "count", "tally", "add_count", "add_tally",
    "pull", "relocate",
    "bind_rows", "bind_cols",
    "left_join", "right_join", "inner_join", "full_join",
    "semi_join", "anti_join", "nest_join", "cross_join",
    "across", "c_across", "everything", "all_of", "any_of",
    "starts_with", "ends_with", "contains", "matches", "num_range",
    "where", "last_col",
    "vars", "one_of",
    "n", "cur_data", "cur_group", "cur_group_id", "cur_group_rows",
    "cur_column", "row_number",
    "desc", "lead", "lag",
    "coalesce", "na_if", "if_else", "case_when", "case_match",
    "between",
    "collect", "compute", "show_query", "explain",
    // dplyr helpers used in internal code
    "enquo", "enquos", "quo_name", "quo_is_null",
    "set_names", "rlang::set_names",
    "pick",
    // tidyr verbs
    "pivot_longer", "pivot_wider",
    "gather", "spread",
    "separate", "separate_rows", "separate_wider_delim",
    "unite", "complete", "drop_na", "fill", "replace_na",
    "nest", "unnest", "unnest_longer", "unnest_wider",
    "expand", "expand_grid", "crossing", "nesting",
    "chop", "unchop", "pack", "unpack",
    // purrr
    "map", "map2", "pmap", "imap",
    "map_lgl", "map_int", "map_dbl", "map_chr", "map_vec",
    "map2_lgl", "map2_int", "map2_dbl", "map2_chr",
    "pmap_lgl", "pmap_int", "pmap_dbl", "pmap_chr",
    "walk", "walk2", "pwalk", "iwalk",
    "accumulate", "reduce", "detect", "detect_index",
    "keep", "discard", "compact",
    "every", "some", "none",
    "possibly", "safely", "quietly",
    "transpose",
    "set_names",
    // lifecycle
    "deprecate_soft", "deprecate_warn", "deprecate_stop",
    "deprecated", "signal_stage",
    "lifecycle::signal_stage", "lifecycle::deprecate_soft",
    "lifecycle::deprecate_warn",
    // glue + cli (always re-exported in tidyverse code)
    "glue", "glue_collapse", "glue_sql", "glue_data",
    "cli_abort", "cli_warn", "cli_inform", "cli_alert",
    "cli_alert_danger", "cli_alert_warning", "cli_alert_info",
    "cli_alert_success", "cli_bullets",
    "cli_text", "cli_h1", "cli_h2", "cli_h3",
    "format_error", "format_warning", "format_message",
    // Common base-R names that appear as bare targets but are part of
    // R's standard evaluation / quoting machinery (the R extractor's
    // resolver doesn't have a base-R index).
    "structure", "quote", "bquote", "substitute", "match.call",
    "sys.call", "sys.function", "parent.frame",
    "NextMethod", "UseMethod",
    "on.exit", "tryCatch",
];

const SHINY_GLOBALS: &[&str] = &[
    "shinyApp", "shinyUI", "shinyServer",
    "fluidPage", "fluidRow", "column",
    "titlePanel", "sidebarLayout", "sidebarPanel", "mainPanel",
    "inputPanel", "fixedPage", "fillPage", "bootstrapPage",
    "tabPanel", "tabsetPanel", "navbarPage", "navbarMenu",
    "wellPanel", "conditionalPanel", "absolutePanel", "fixedPanel",
    "textInput", "numericInput", "selectInput", "selectizeInput",
    "checkboxInput", "checkboxGroupInput", "radioButtons",
    "sliderInput", "dateInput", "dateRangeInput", "fileInput",
    "textOutput", "verbatimTextOutput", "plotOutput", "tableOutput",
    "uiOutput", "htmlOutput", "imageOutput",
    "renderText", "renderPrint", "renderPlot", "renderTable",
    "renderUI", "renderImage",
    "reactive", "reactiveVal", "reactiveValues",
    "observe", "observeEvent",
    "isolate", "invalidateLater",
    "req", "validate", "need",
    "eventReactive",
    "updateTextInput", "updateSelectInput", "updateSliderInput",
    "updateNumericInput", "updateCheckboxInput",
    "showModal", "removeModal", "modalDialog", "modalButton",
    "showNotification", "removeNotification",
    "runApp", "shinyOptions",
    "session",
];
