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
    if deps.contains("rlang") {
        globals.extend(RLANG_GLOBALS);
    }
    if deps.contains("glue") {
        globals.extend(&["glue", "glue_collapse", "glue_sql", "glue_data"]);
    }
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
