// =============================================================================
// cmake/primitives.rs — CMake primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for CMake.
pub(crate) const PRIMITIVES: &[&str] = &[
    // project setup
    "project", "cmake_minimum_required",
    // targets
    "add_executable", "add_library", "add_custom_target",
    "target_link_libraries", "target_include_directories",
    "target_compile_definitions", "target_compile_options",
    "target_compile_features", "target_sources",
    // find
    "find_package", "find_library", "find_path", "find_program", "find_file",
    // inclusion
    "include", "include_directories", "link_directories", "link_libraries",
    // variables
    "set", "unset", "get_property", "set_property",
    "get_target_property", "set_target_properties", "option",
    // messages
    "message",
    // flow
    "if", "elseif", "else", "endif",
    "foreach", "endforeach",
    "while", "endwhile",
    "function", "endfunction",
    "macro", "endmacro",
    "return", "break", "continue",
    // commands
    "add_custom_command", "add_subdirectory", "add_definitions",
    "add_dependencies", "add_test",
    "enable_testing", "install",
    // file / string / list / math
    "configure_file", "file", "string", "list", "math",
    "execute_process",
    "cmake_parse_arguments", "get_filename_component",
    "mark_as_advanced", "separate_arguments", "site_name",
    "variable_watch", "cmake_path", "block", "endblock",
    "cmake_policy", "cmake_host_system_information",
    // FetchContent / ExternalProject
    "FetchContent_Declare", "FetchContent_MakeAvailable",
    "FetchContent_Populate", "FetchContent_GetProperties",
    "ExternalProject_Add", "CPMAddPackage",
    // boolean literals
    "TRUE", "FALSE", "ON", "OFF", "YES", "NO",
    // scope / visibility
    "CACHE", "PARENT_SCOPE", "GLOBAL",
    "INTERFACE", "PUBLIC", "PRIVATE",
    // find_package options
    "REQUIRED", "QUIET", "CONFIG", "MODULE", "COMPONENTS",
    "IMPORTED", "ALIAS",
    // target types
    "OBJECT", "STATIC", "SHARED",
    // message types
    "FATAL_ERROR", "SEND_ERROR", "WARNING", "AUTHOR_WARNING",
    "DEPRECATION", "STATUS", "VERBOSE", "DEBUG", "TRACE",
    "CHECK_START", "CHECK_PASS", "CHECK_FAIL",
    // condition operators
    "NOT", "AND", "OR", "DEFINED", "EQUAL", "LESS", "GREATER",
    "STREQUAL", "MATCHES",
    "VERSION_EQUAL", "VERSION_LESS", "VERSION_GREATER",
    "EXISTS", "IS_DIRECTORY", "IS_ABSOLUTE", "COMMAND",
];
