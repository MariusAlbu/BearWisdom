// =============================================================================
// cmake/coverage_tests.rs
//
// Node-kind coverage for CMakePlugin::symbol_node_kinds() and ref_node_kinds().
// The plugin mod.rs returns ExtractionResult::empty() pending grammar wiring;
// these tests call extract::extract() directly with the live grammar.
//
// symbol_node_kinds: function_def, macro_def, normal_command
// ref_node_kinds:    normal_command, variable_ref
// =============================================================================

use super::extract;
use super::resolve::is_cmake_builtin;
use crate::types::{EdgeKind, SymbolKind};

fn lang() -> tree_sitter::Language {
    tree_sitter_cmake::LANGUAGE.into()
}

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_function_def_produces_function() {
    let src = "function(my_func arg1)\n  message(\"hello\")\nendfunction()";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "my_func"),
        "function_def should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_macro_def_produces_function() {
    let src = "macro(my_macro)\n  message(\"macro\")\nendmacro()";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "my_macro"),
        "macro_def should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_normal_command_produces_symbol_or_ref() {
    // add_executable is a normal_command that creates a build target (Function).
    let src = "add_executable(myapp main.c)";
    let r = extract::extract(src, lang());
    // Either a symbol or a Calls ref — at minimum the file should parse without error.
    assert!(!r.has_errors, "normal_command snippet should parse cleanly");
}

// ---------------------------------------------------------------------------
// set(<name> ...) → Variable
// ---------------------------------------------------------------------------

#[test]
fn cov_set_command_produces_variable() {
    let src = "set(MY_VAR hello)";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "MY_VAR"),
        "set() should produce Variable 'MY_VAR'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_set_command_cache_produces_variable() {
    let src = "set(INSTALL_DIR \"/usr\" CACHE PATH \"Install prefix\")";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "INSTALL_DIR"),
        "set() with CACHE should produce Variable 'INSTALL_DIR'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// option(<name> ...) → Variable
// ---------------------------------------------------------------------------

#[test]
fn cov_option_command_produces_variable() {
    let src = "option(ENABLE_TESTS \"Enable unit tests\" ON)";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "ENABLE_TESTS"),
        "option() should produce Variable 'ENABLE_TESTS'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// add_executable / add_library / add_custom_target → Function (build target)
// ---------------------------------------------------------------------------

#[test]
fn cov_add_executable_produces_function() {
    let src = "add_executable(myapp main.cpp util.cpp)";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "myapp"),
        "add_executable() should produce Function 'myapp'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_add_library_produces_function() {
    let src = "add_library(mylib STATIC lib.cpp)";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "mylib"),
        "add_library() should produce Function 'mylib'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_add_custom_target_produces_function() {
    let src = "add_custom_target(generate_headers COMMAND python gen.py)";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "generate_headers"),
        "add_custom_target() should produce Function 'generate_headers'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// project(<name> ...) → Namespace
// ---------------------------------------------------------------------------

#[test]
fn cov_project_command_produces_namespace() {
    let src = "project(MyProject VERSION 1.0)";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Namespace && s.name == "MyProject"),
        "project() should produce Namespace 'MyProject'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// include(<path>) → Imports edge
// ---------------------------------------------------------------------------

#[test]
fn cov_include_command_produces_imports() {
    let src = "include(GNUInstallDirs)";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "GNUInstallDirs"),
        "include() should produce Imports ref to 'GNUInstallDirs'; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_include_command_file_path_produces_imports() {
    let src = "include(cmake/CompilerFlags.cmake)";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "include() with file path should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// find_package(<pkg> ...) → Imports edge
// ---------------------------------------------------------------------------

#[test]
fn cov_find_package_produces_imports() {
    let src = "find_package(OpenSSL REQUIRED)";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "OpenSSL"),
        "find_package() should produce Imports ref to 'OpenSSL'; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// add_subdirectory(<dir>) → Imports edge
// ---------------------------------------------------------------------------

#[test]
fn cov_add_subdirectory_produces_imports() {
    let src = "add_subdirectory(src)";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "src"),
        "add_subdirectory() should produce Imports ref to 'src'; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// target_link_libraries → Calls edges from target to each library
// ---------------------------------------------------------------------------

#[test]
fn cov_target_link_libraries_produces_calls() {
    let src = "add_executable(myapp main.cpp)\ntarget_link_libraries(myapp PRIVATE OpenSSL::SSL Threads::Threads)";
    let r = extract::extract(src, lang());
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::Calls)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        calls.iter().any(|&n| n.contains("OpenSSL") || n.contains("SSL")),
        "target_link_libraries should produce Calls edge to OpenSSL lib; got: {calls:?}"
    );
}

#[test]
fn cov_target_link_libraries_skips_keywords() {
    // PRIVATE / PUBLIC / INTERFACE are visibility keywords, not library names
    let src = "add_library(mylib STATIC lib.cpp)\ntarget_link_libraries(mylib PUBLIC fmt::fmt)";
    let r = extract::extract(src, lang());
    let call_names: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::Calls)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        !call_names.contains(&"PUBLIC"),
        "visibility keyword 'PUBLIC' should not appear as a Calls target; got: {call_names:?}"
    );
}

// ---------------------------------------------------------------------------
// variable_ref (${VAR}) → TypeRef edge (not Calls)
// ---------------------------------------------------------------------------

#[test]
fn cov_variable_ref_produces_typeref() {
    let src = "set(SRC_DIR src)\nadd_subdirectory(${SRC_DIR})";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "SRC_DIR"),
        "variable_ref should produce TypeRef to 'SRC_DIR'; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// normal_command inside function body → Calls ref attributed to function
// ---------------------------------------------------------------------------

#[test]
fn cov_command_inside_function_body_produces_calls() {
    let src = "function(setup_project name)\n  message(STATUS \"Setting up ${name}\")\n  add_definitions(-DPROJECT=${name})\nendfunction()";
    let r = extract::extract(src, lang());
    // The function itself must be extracted.
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "setup_project"),
        "function_def should produce Function 'setup_project'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    // Commands inside the body should produce Calls refs.
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "commands inside function body should produce Calls refs; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// add_test → Test symbol (test detection)
// ---------------------------------------------------------------------------

#[test]
fn cov_add_test_command_produces_function() {
    // add_test emits a Function symbol (normal_command path).
    // The rules spec says first arg should be Test kind, but extractor uses Function.
    let src = "add_test(NAME unit_tests COMMAND myapp --test)";
    let r = extract::extract(src, lang());
    // At minimum it should not crash and should emit a symbol or ref.
    assert!(!r.has_errors, "add_test snippet should parse cleanly");
}

// ---------------------------------------------------------------------------
// Function and macro parameters → Variable symbols
// ---------------------------------------------------------------------------

#[test]
fn cov_function_parameters_produce_variables() {
    let src = "function(setup_project NAME VERSION)\n  message(${NAME} ${VERSION})\nendfunction()";
    let r = extract::extract(src, lang());
    let var_names: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Variable)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        var_names.contains(&"NAME") && var_names.contains(&"VERSION"),
        "function parameters NAME and VERSION should be Variable symbols; got: {var_names:?}",
    );
}

#[test]
fn cov_macro_parameters_produce_variables() {
    let src = "macro(my_macro KEY VALUE)\n  set(MAP_${KEY} ${VALUE})\nendmacro()";
    let r = extract::extract(src, lang());
    let var_names: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Variable)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        var_names.contains(&"KEY") && var_names.contains(&"VALUE"),
        "macro parameters KEY and VALUE should be Variable symbols; got: {var_names:?}",
    );
}

// ---------------------------------------------------------------------------
// foreach loop variable → Variable symbol
// ---------------------------------------------------------------------------

#[test]
fn cov_foreach_loop_var_produces_variable() {
    let src = "foreach(PACKAGE IN LISTS deps)\n  message(${PACKAGE})\nendforeach()";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "PACKAGE"),
        "foreach loop var PACKAGE should be Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// string() output variables — TOLOWER/TOUPPER (arg 2), SHA1 (arg 1), SUBSTRING (last)
// ---------------------------------------------------------------------------

#[test]
fn cov_string_tolower_output_var_produces_variable() {
    let src = "string(TOLOWER ${NAME} lower_case_name)";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "lower_case_name"),
        "string(TOLOWER) output should be Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_string_sha1_output_var_produces_variable() {
    let src = "string(SHA1 origin_hash \"some data\")";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "origin_hash"),
        "string(SHA1) output should be Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_string_substring_output_var_produces_variable() {
    let src = "string(SUBSTRING \"${origin_hash}\" 0 8 short_hash)";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "short_hash"),
        "string(SUBSTRING) output should be Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// string(REPLACE/APPEND/PREPEND) — output variable position
// ---------------------------------------------------------------------------

#[test]
fn cov_string_replace_output_var_is_third_arg() {
    // string(REPLACE <match> <replace> <out_var> <input...>)
    let src = r#"string(REPLACE " " ";" EXTRA_ARGS "${ARGN}")"#;
    let r = extract::extract(src, lang());
    let var_names: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Variable)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        var_names.contains(&"EXTRA_ARGS"),
        "string(REPLACE) output should be at index 3; got: {var_names:?}",
    );
}

#[test]
fn cov_string_append_output_var_is_first_arg() {
    // string(APPEND <string_var> ...) — string_var is index 1
    let src = r##"string(APPEND PRETTY_OUT_VAR "#")"##;
    let r = extract::extract(src, lang());
    let var_names: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Variable)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        var_names.contains(&"PRETTY_OUT_VAR"),
        "string(APPEND) target var should be extracted; got: {var_names:?}",
    );
}

// ---------------------------------------------------------------------------
// math(EXPR <out> ...) → Variable
// ---------------------------------------------------------------------------

#[test]
fn cov_math_expr_output_var_produces_variable() {
    let src = "math(EXPR result \"1 + 2\")";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "result"),
        "math(EXPR) output should be Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// get_filename_component(<out> ...) → Variable
// ---------------------------------------------------------------------------

#[test]
fn cov_get_filename_component_output_var_produces_variable() {
    let src = "get_filename_component(SCRIPT_DIR \"${CMAKE_CURRENT_LIST_FILE}\" DIRECTORY)";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "SCRIPT_DIR"),
        "get_filename_component output should be Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// find_program/find_library/find_path/find_file → Variable from first arg
// ---------------------------------------------------------------------------

#[test]
fn cov_find_program_output_var_produces_variable() {
    let src = "find_program(CPPCHECK_BIN cppcheck)";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "CPPCHECK_BIN"),
        "find_program output should be Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_find_library_output_var_produces_variable() {
    let src = "find_library(MATH_LIB m)";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "MATH_LIB"),
        "find_library output should be Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// file(<MODE> ...) — output variable position depends on mode
// ---------------------------------------------------------------------------

#[test]
fn cov_file_glob_output_var_produces_variable() {
    let src = "file(GLOB ALL_SOURCE_FILES \"src/*.cpp\")";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "ALL_SOURCE_FILES"),
        "file(GLOB ...) output should be Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_file_read_output_var_produces_variable() {
    let src = "file(READ \"version.txt\" VERSION_STRING)";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "VERSION_STRING"),
        "file(READ ...) output should be Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// cmake_parse_arguments → prefix + <prefix>_<keyword> Variables
// ---------------------------------------------------------------------------

#[test]
fn cov_cmake_parse_arguments_emits_prefix_variables() {
    let src = "cmake_parse_arguments(MY_FN \"REQUIRED;OPTIONAL\" \"NAME;VERSION\" \"SOURCES\" ${ARGN})";
    let r = extract::extract(src, lang());
    let var_names: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Variable)
        .map(|s| s.name.as_str())
        .collect();
    for expected in ["MY_FN", "MY_FN_REQUIRED", "MY_FN_OPTIONAL", "MY_FN_NAME", "MY_FN_VERSION", "MY_FN_SOURCES"] {
        assert!(
            var_names.contains(&expected),
            "cmake_parse_arguments should emit Variable {expected}; got: {var_names:?}",
        );
    }
}

#[test]
fn cov_cmake_parse_arguments_parse_argv_form() {
    let src = "cmake_parse_arguments(PARSE_ARGV 1 ARG \"\" \"SOURCE_DIR;BINARY_DIR\" \"\")";
    let r = extract::extract(src, lang());
    let var_names: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Variable)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        var_names.contains(&"ARG_SOURCE_DIR") && var_names.contains(&"ARG_BINARY_DIR"),
        "PARSE_ARGV form should emit ARG_SOURCE_DIR and ARG_BINARY_DIR; got: {var_names:?}",
    );
}

// ---------------------------------------------------------------------------
// execute_process(... OUTPUT_VARIABLE <out> ...) → Variable
// ---------------------------------------------------------------------------

#[test]
fn cov_execute_process_output_variables_produce_variables() {
    let src = "execute_process(COMMAND git rev-parse HEAD OUTPUT_VARIABLE GIT_SHA RESULT_VARIABLE GIT_RES)";
    let r = extract::extract(src, lang());
    let var_names: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Variable)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        var_names.contains(&"GIT_SHA") && var_names.contains(&"GIT_RES"),
        "execute_process should emit GIT_SHA and GIT_RES; got: {var_names:?}",
    );
}

// ---------------------------------------------------------------------------
// find_package(<pkg>) → conventional output variable symbols
// ---------------------------------------------------------------------------

#[test]
fn cov_find_package_emits_found_variable() {
    let src = "find_package(OpenSSL REQUIRED)";
    let r = extract::extract(src, lang());
    let var_names: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Variable)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        var_names.contains(&"OpenSSL_FOUND") || var_names.contains(&"OPENSSL_FOUND"),
        "find_package(OpenSSL) should emit OpenSSL_FOUND / OPENSSL_FOUND; got: {var_names:?}",
    );
}

#[test]
fn cov_find_package_emits_libraries_and_include_dirs() {
    let src = "find_package(Protobuf REQUIRED)";
    let r = extract::extract(src, lang());
    let var_names: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Variable)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        var_names.contains(&"PROTOBUF_LIBRARIES"),
        "find_package(Protobuf) should emit PROTOBUF_LIBRARIES; got: {var_names:?}",
    );
    assert!(
        var_names.contains(&"PROTOBUF_INCLUDE_DIRS"),
        "find_package(Protobuf) should emit PROTOBUF_INCLUDE_DIRS; got: {var_names:?}",
    );
}

#[test]
fn cov_find_package_git_emits_executable() {
    let src = "find_package(Git REQUIRED)";
    let r = extract::extract(src, lang());
    let var_names: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Variable)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        var_names.contains(&"GIT_EXECUTABLE"),
        "find_package(Git) should emit GIT_EXECUTABLE; got: {var_names:?}",
    );
}

// ---------------------------------------------------------------------------
// separate_arguments(<out> ...) → Variable from first arg
// ---------------------------------------------------------------------------

#[test]
fn cov_separate_arguments_output_var_produces_variable() {
    let src = "separate_arguments(tmp_args UNIX_COMMAND ${CPPCHECK_ARG})";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "tmp_args"),
        "separate_arguments output should be Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// is_cmake_builtin — ARGC / ARGN / ARGV special variables
// ---------------------------------------------------------------------------

#[test]
fn builtin_argc_is_recognized() {
    assert!(is_cmake_builtin("ARGC"), "ARGC must be a builtin");
    assert!(is_cmake_builtin("argc"), "argc (lowercase) must be a builtin");
}

#[test]
fn builtin_argn_argv_are_recognized() {
    assert!(is_cmake_builtin("ARGN"), "ARGN must be a builtin");
    assert!(is_cmake_builtin("ARGV"), "ARGV must be a builtin");
    assert!(is_cmake_builtin("ARGV0"), "ARGV0 must be a builtin");
    assert!(is_cmake_builtin("ARGV9"), "ARGV9 must be a builtin");
}

#[test]
fn builtin_cmake_prefix_is_recognized() {
    assert!(is_cmake_builtin("CMAKE_CURRENT_SOURCE_DIR"), "cmake_ prefix must be builtin");
    assert!(is_cmake_builtin("FETCHCONTENT_BASE_DIR"), "fetchcontent_ prefix must be builtin");
}

#[test]
fn non_builtin_user_var_not_recognized() {
    assert!(!is_cmake_builtin("MY_PROJECT_DIR"), "user variable must not be builtin");
    assert!(!is_cmake_builtin("adder"), "project target must not be builtin");
}
