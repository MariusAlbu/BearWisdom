// =============================================================================
// cmake/keywords.rs — CMake built-in commands, scope tokens, and keyword args
//
// CMake is case-insensitive at the command level, so all entries are lowercase
// and callers must lowercase input before lookup. The list comes from the
// resolver's `is_cmake_builtin()` exact-match branch (prefix matching stays in
// resolve.rs because `cmake_*`, `project_*`, `ctest_*`, `cpack_*`,
// `fetchcontent_*`, and `argv<N>` are open-ended patterns, not enumerable).
// =============================================================================

pub(crate) const KEYWORDS: &[&str] = &[
    // Control flow
    "if", "else", "elseif", "endif",
    "while", "endwhile",
    "foreach", "endforeach",
    "function", "endfunction",
    "macro", "endmacro",
    "return", "break", "continue",
    // Configuration / project
    "cmake_minimum_required", "project", "cmake_policy",
    "cmake_parse_arguments", "cmake_language",
    // Target commands
    "add_executable", "add_library", "add_custom_target",
    "add_custom_command", "add_test", "add_subdirectory",
    "add_dependencies", "add_compile_options", "add_compile_definitions",
    "add_link_options",
    // Property commands
    "set_target_properties", "get_target_property",
    "set_property", "get_property",
    "target_compile_options", "target_compile_definitions",
    "target_include_directories", "target_link_libraries",
    "target_link_options", "target_sources",
    "target_compile_features", "target_precompile_headers",
    // Find commands
    "find_package", "find_library", "find_program",
    "find_path", "find_file",
    // Variable / cache
    "set", "unset", "option", "list", "string", "math",
    "message", "configure_file", "file", "include",
    "include_directories", "link_directories", "link_libraries",
    // Install
    "install", "export",
    // Testing
    "enable_testing", "ctest_configure", "ctest_build",
    "ctest_test", "set_tests_properties",
    // String / list / misc
    "separate_arguments", "include_guard",
    // Misc
    "execute_process", "try_compile", "try_run",
    "define_property", "mark_as_advanced",
    "source_group", "aux_source_directory",
    "enable_language", "get_filename_component",
    "check_include_file", "check_function_exists",
    "check_symbol_exists", "check_library_exists",
    "check_cxx_source_compiles", "check_c_source_compiles",
    "check_type_size", "check_struct_has_member",
    // CPM.cmake
    "cpmaddpackage",
    "cpmfindpackage",
    // Common cmake keyword arguments / scope tokens (appear as args, not commands,
    // but may leak as Calls targets from target_link_libraries argument lists)
    "cache", "internal", "bool", "path", "filepath",
    "force", "docstring",
    "interface", "public", "private",
    "link_public", "link_private",
    "static", "shared", "module", "object", "alias",
    "required", "quiet", "config", "module_", "components",
    "imported", "global", "parent_scope",
    "fatal_error", "send_error", "warning", "author_warning",
    "deprecation", "status", "verbose", "debug", "trace",
    "check_start", "check_pass", "check_fail",
    "not", "and", "or", "defined", "equal", "less", "greater",
    "strequal", "matches",
    "version_equal", "version_less", "version_greater",
    "version_less_equal", "version_greater_equal",
    "exists", "is_directory", "is_absolute",
    "name", "command", "args", "append", "prepend",
    "on", "off", "true", "false", "yes", "no",
    "win32", "apple", "unix", "msvc", "mingw", "ios", "android",
];
