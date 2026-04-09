// =============================================================================
// robot/builtins.rs — Robot Framework BuiltIn library keywords
// =============================================================================

use crate::types::EdgeKind;

/// Normalize a Robot Framework keyword name for comparison.
/// Robot treats spaces and underscores as equivalent and is case-insensitive.
pub(super) fn normalize_robot_name(name: &str) -> String {
    name.to_ascii_lowercase()
        .chars()
        .map(|c| if c == ' ' || c == '_' { '_' } else { c })
        .collect()
}

/// Edge-kind / symbol-kind compatibility for Robot Framework.
#[allow(dead_code)]
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "function" | "method"),
        _ => true,
    }
}

/// Well-known Robot Framework library names (external, not project code).
/// Used to classify qualified `Library.Keyword` references as external.
pub(super) fn is_robot_builtin_library(name: &str) -> bool {
    let norm = normalize_robot_name(name);
    matches!(
        norm.as_str(),
        "builtin"
            | "collections"
            | "string"
            | "operatingsystem"
            | "process"
            | "datetime"
            | "xml"
            | "json"
            | "requestslibrary"
            | "seleniumlibrary"
            | "appiumlibrary"
            | "playwrightlibrary"
            | "browserlibrary"
            | "ftplibrary"
            | "imaplibrary"
            | "databaselibrary"
            | "exceldatalibrary"
            | "arquillian"
            | "robotframework_requests"
            | "robotframework_selenium2library"
    )
}

/// Robot Framework BuiltIn library keywords and SeleniumLibrary keywords.
pub(super) fn is_robot_builtin(name: &str) -> bool {
    let normalized = normalize_robot_name(name);
    matches!(
        normalized.as_str(),
        // -----------------------------------------------------------------------
        // Logging
        // -----------------------------------------------------------------------
        "log"
            | "log_many"
            | "log_to_console"
            | "log_variables"
            // -----------------------------------------------------------------------
            // Variable handling
            // -----------------------------------------------------------------------
            | "set_variable"
            | "set_suite_variable"
            | "set_global_variable"
            | "set_test_variable"
            | "set_local_variable"
            | "set_task_variable"
            | "get_variable_value"
            | "variable_should_exist"
            | "variable_should_not_exist"
            // -----------------------------------------------------------------------
            // Assertions — equality / truth
            // -----------------------------------------------------------------------
            | "should_be_equal"
            | "should_not_be_equal"
            | "should_be_true"
            | "should_not_be_true"
            | "should_be_empty"
            | "should_not_be_empty"
            | "should_contain"
            | "should_not_contain"
            | "should_start_with"
            | "should_end_with"
            | "should_match"
            | "should_not_match"
            | "should_match_regexp"
            | "should_not_match_regexp"
            | "should_be_equal_as_integers"
            | "should_be_equal_as_numbers"
            | "should_be_equal_as_strings"
            | "should_be_equal_as_bytes"
            | "should_not_be_equal_as_integers"
            | "should_not_be_equal_as_numbers"
            | "should_not_be_equal_as_strings"
            | "length_should_be"
            // -----------------------------------------------------------------------
            // Control flow — run keyword variants
            // -----------------------------------------------------------------------
            | "run_keyword"
            | "run_keyword_if"
            | "run_keyword_unless"
            | "run_keyword_and_return"
            | "run_keyword_and_return_if"
            | "run_keyword_and_return_status"
            | "run_keyword_and_ignore_error"
            | "run_keyword_and_expect_error"
            | "run_keyword_and_continue_on_failure"
            | "run_keyword_and_warn_on_failure"
            | "run_keywords"
            | "repeat_keyword"
            | "wait_until_keyword_succeeds"
            | "run_keyword_if_any_tests_failed"
            | "run_keyword_if_all_tests_passed"
            | "run_keyword_if_any_critical_tests_failed"
            | "run_keyword_if_all_critical_tests_passed"
            | "run_keyword_if_test_failed"
            | "run_keyword_if_test_passed"
            | "run_keyword_if_timeout_occurred"
            | "run_keyword_if_setup_failed"
            | "run_keyword_if_teardown_failed"
            | "return_from_keyword"
            | "return_from_keyword_if"
            // -----------------------------------------------------------------------
            // Execution control
            // -----------------------------------------------------------------------
            | "pass_execution"
            | "pass_execution_if"
            | "fatal_error"
            | "fail"
            | "skip"
            | "skip_if"
            | "comment"
            | "no_operation"
            | "sleep"
            // -----------------------------------------------------------------------
            // Type conversion
            // -----------------------------------------------------------------------
            | "convert_to_integer"
            | "convert_to_string"
            | "convert_to_number"
            | "convert_to_boolean"
            | "convert_to_bytes"
            | "convert_to_hex"
            | "convert_to_octal"
            // -----------------------------------------------------------------------
            // Collections
            // -----------------------------------------------------------------------
            | "create_list"
            | "create_dictionary"
            | "append_to_list"
            | "remove_from_list"
            | "get_from_list"
            | "get_from_dictionary"
            | "set_to_dictionary"
            // -----------------------------------------------------------------------
            // String / evaluation
            // -----------------------------------------------------------------------
            | "evaluate"
            | "call_method"
            | "catenate"
            | "get_time"
            | "get_count"
            | "get_length"
            // -----------------------------------------------------------------------
            // Library / resource management
            // -----------------------------------------------------------------------
            | "import_library"
            | "import_resource"
            | "import_variables"
            | "set_library_search_order"
            | "keyword_should_exist"
            | "set_log_level"
            // -----------------------------------------------------------------------
            // SeleniumLibrary keywords
            // -----------------------------------------------------------------------
            | "open_browser"
            | "close_browser"
            | "close_all_browsers"
            | "go_to"
            | "click_element"
            | "click_button"
            | "click_link"
            | "input_text"
            | "select_from_list_by_value"
            | "select_from_list_by_label"
            | "select_from_list_by_index"
            | "wait_until_element_is_visible"
            | "wait_until_element_is_enabled"
            | "wait_until_page_contains"
            | "page_should_contain"
            | "page_should_not_contain"
            | "element_should_be_visible"
            | "element_should_not_be_visible"
            | "element_should_contain"
            | "element_should_not_contain"
            | "get_text"
            | "get_title"
            | "get_location"
            | "get_element_attribute"
            | "get_webelement"
            | "get_webelements"
            | "execute_javascript"
            | "capture_page_screenshot"
            | "maximize_browser_window"
            | "set_selenium_speed"
            | "set_selenium_timeout"
            | "reload_page"
    )
}
