// =============================================================================
// make/primitives.rs — GNU Make primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for GNU Make.
pub(crate) const PRIMITIVES: &[&str] = &[
    // make built-in functions
    "subst", "patsubst", "strip", "findstring",
    "filter", "filter-out", "sort", "word", "words",
    "wordlist", "firstword", "lastword",
    "dir", "notdir", "suffix", "basename", "addsuffix",
    "addprefix", "join", "wildcard", "realpath", "abspath",
    "if", "or", "and", "foreach", "file", "call", "value",
    "eval", "origin", "flavor", "error", "warning", "info",
    "shell", "guile",
    // built-in variables
    "MAKE", "MAKEFILE_LIST", "MAKEFLAGS", "MAKEOVERRIDES",
    "MAKECMDGOALS", "CURDIR", "VPATH",
    "CC", "CXX", "LD", "AR", "AS", "FC", "PC",
    "CFLAGS", "CXXFLAGS", "LDFLAGS", "LDLIBS", "LIBS",
    "ARFLAGS", "ASFLAGS", "FFLAGS",
    "CPPFLAGS", "TARGET_ARCH",
    "COMPILE.c", "COMPILE.cc", "COMPILE.cpp",
    "LINK.c", "LINK.cc", "LINK.o",
    "OUTPUT_OPTION",
    ".DEFAULT_GOAL", ".PHONY", ".SUFFIXES", ".PRECIOUS",
    ".INTERMEDIATE", ".SECONDARY", ".DELETE_ON_ERROR",
    ".IGNORE", ".SILENT", ".NOTPARALLEL",
    ".EXPORT_ALL_VARIABLES", ".LOW_RESOLUTION_TIME",
    // automatic variables
    "$@", "$<", "$^", "$+", "$?", "$*", "$%",
    "$(@D)", "$(@F)", "$(<D)", "$(<F)", "$(^D)", "$(^F)",
    // common shell commands used in recipes
    "echo", "printf", "cp", "mv", "rm", "mkdir", "rmdir",
    "touch", "chmod", "chown", "ln", "cat", "grep", "sed",
    "awk", "find", "xargs", "tar", "gzip", "install",
    "true", "false", "test", "[",
    "gcc", "g++", "clang", "clang++", "cc",
    "ar", "ranlib", "strip", "objcopy", "objdump",
    "ld", "nm", "size",
    "python", "python3", "pip", "node", "npm",
    "go", "cargo", "rustc",
    "git", "curl", "wget",
    "pkg-config",
    "@", "-", "+",
];
