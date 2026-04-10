// =============================================================================
// c_lang/builtins.rs — C/C++ builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "struct"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "struct" | "interface" | "enum" | "type_alias" | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "struct"),
        _ => true,
    }
}

/// Always-external C/C++ namespace roots (std, boost, test frameworks).
const ALWAYS_EXTERNAL_NAMESPACES: &[&str] = &["std", "boost", "gtest", "gmock", "catch2"];

/// Check whether a C/C++ namespace is external.
pub(super) fn is_external_c_namespace(ns: &str) -> bool {
    // Strip leading `::` (global scope qualifier).
    let ns = ns.strip_prefix("::").unwrap_or(ns);
    let root = ns.split("::").next().unwrap_or(ns);
    for prefix in ALWAYS_EXTERNAL_NAMESPACES {
        if root == *prefix {
            return true;
        }
    }
    false
}

/// C standard library header names (used to classify `#include <header>`).
const C_STDLIB_HEADERS: &[&str] = &[
    "stdio", "stdlib", "string", "math", "assert", "ctype", "errno",
    "float", "limits", "locale", "setjmp", "signal", "stdarg", "stddef",
    "stdint", "inttypes", "time", "wchar", "wctype", "stdbool", "complex",
    "tgmath", "fenv", "iso646", "threads", "uchar",
    // C++ standard headers (common subset)
    "algorithm", "array", "bitset", "chrono", "codecvt", "complex",
    "condition_variable", "deque", "exception", "filesystem", "forward_list",
    "fstream", "functional", "future", "initializer_list", "iomanip", "ios",
    "iosfwd", "iostream", "istream", "iterator", "limits", "list", "locale",
    "map", "memory", "mutex", "new", "numeric", "optional", "ostream",
    "queue", "random", "ratio", "regex", "set", "shared_mutex", "sstream",
    "stack", "stdexcept", "streambuf", "string", "string_view", "system_error",
    "thread", "tuple", "type_traits", "typeindex", "typeinfo", "unordered_map",
    "unordered_set", "utility", "valarray", "variant", "vector",
    // POSIX headers
    "unistd", "fcntl", "sys/types", "sys/stat", "sys/socket", "netinet/in",
    "arpa/inet", "netdb", "dirent", "pthread", "semaphore",
];

/// Check whether a `#include` path is a system/stdlib header.
pub(super) fn is_system_header(path: &str) -> bool {
    // System headers use angle brackets — the extractor typically marks these
    // differently, but we check by stripped name as a fallback.
    let base = path
        .trim_matches(|c| c == '<' || c == '>' || c == '"')
        .split('/')
        .last()
        .unwrap_or(path);
    // Strip `.h` suffix.
    let base = base.strip_suffix(".h").unwrap_or(base);
    for &h in C_STDLIB_HEADERS {
        if base == h {
            return true;
        }
    }
    false
}

/// Template parameter names and patterns that should be classified as external
/// (they're not real symbols in the index, just formal type parameters).
pub(super) fn is_template_param(name: &str) -> bool {
    // Synthetic token emitted for template_argument_list coverage nodes.
    if name == "<template_args>" {
        return true;
    }
    // Single uppercase letter: T, U, V, K, N, E, etc.
    if name.len() == 1 && name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        return true;
    }
    // Names ending in "Type" are almost always template type parameters
    // (e.g. BasicJsonType, CharType, IteratorType, KeyType, ValueType).
    if name.ends_with("Type") && name.len() > 4 {
        return true;
    }
    // Common multi-character template param names.
    matches!(
        name,
        "Args"
            | "Func"
            | "Allocator"
            | "Iterator"
            | "Container"
            | "Predicate"
            | "Compare"
            | "Hash"
            | "KeyEqual"
            | "Traits"
            | "CharT"
            | "Tp"
            | "Up"
            | "Vp"
            | "Ts"
            | "Us"
            | "Vs"
    )
}

/// C/C++ builtins and common names that are never in the project index.
pub(super) fn is_c_builtin(name: &str) -> bool {
    // Strip `std::` prefix — `std::string` root is still a stdlib type.
    let root = name
        .strip_prefix("std::")
        .unwrap_or(name)
        .split("::")
        .next()
        .unwrap_or(name);

    matches!(
        root,
        // C stdlib functions
        "printf"
            | "fprintf"
            | "sprintf"
            | "snprintf"
            | "scanf"
            | "fscanf"
            | "sscanf"
            | "malloc"
            | "calloc"
            | "realloc"
            | "free"
            | "memcpy"
            | "memset"
            | "memmove"
            | "memcmp"
            | "strcmp"
            | "strncmp"
            | "strlen"
            | "strcpy"
            | "strncpy"
            | "strcat"
            | "strncat"
            | "strstr"
            | "strchr"
            | "atoi"
            | "atof"
            | "atol"
            | "strtol"
            | "strtod"
            | "strtoul"
            | "qsort"
            | "bsearch"
            | "abs"
            | "labs"
            | "fabs"
            | "ceil"
            | "floor"
            | "round"
            | "sqrt"
            | "pow"
            | "sin"
            | "cos"
            | "tan"
            | "log"
            | "exp"
            | "fmod"
            | "exit"
            | "abort"
            | "atexit"
            | "getenv"
            | "system"
            | "rand"
            | "srand"
            | "time"
            | "clock"
            // C++ keywords / operators used as symbol refs
            | "sizeof"
            | "alignof"
            | "typeid"
            | "nullptr"
            | "new"
            | "delete"
            | "this"
            | "super"
            // STL types in the std namespace (stripped prefix)
            | "string"
            | "wstring"
            | "u16string"
            | "u32string"
            | "string_view"
            | "vector"
            | "list"
            | "deque"
            | "forward_list"
            | "array"
            | "stack"
            | "queue"
            | "priority_queue"
            | "map"
            | "multimap"
            | "unordered_map"
            | "unordered_multimap"
            | "set"
            | "multiset"
            | "unordered_set"
            | "unordered_multiset"
            | "pair"
            | "tuple"
            | "optional"
            | "variant"
            | "any"
            | "function"
            | "shared_ptr"
            | "unique_ptr"
            | "weak_ptr"
            | "make_shared"
            | "make_unique"
            | "thread"
            | "mutex"
            | "recursive_mutex"
            | "lock_guard"
            | "unique_lock"
            | "condition_variable"
            | "future"
            | "promise"
            | "async"
            | "atomic"
            | "bitset"
            | "regex"
            | "smatch"
            | "cmatch"
            | "exception"
            | "runtime_error"
            | "logic_error"
            | "invalid_argument"
            | "out_of_range"
            | "overflow_error"
            | "bad_alloc"
            | "ios_base"
            | "istream"
            | "ostream"
            | "iostream"
            | "ifstream"
            | "ofstream"
            | "fstream"
            | "stringstream"
            | "istringstream"
            | "ostringstream"
            | "cin"
            | "cout"
            | "cerr"
            | "clog"
            | "endl"
            | "flush"
            // STL algorithms / utilities (free functions in <algorithm>, <utility>)
            | "sort"
            | "stable_sort"
            | "find"
            | "find_if"
            | "count"
            | "count_if"
            | "copy"
            | "copy_if"
            | "transform"
            | "for_each"
            | "accumulate"
            | "reduce"
            | "all_of"
            | "any_of"
            | "none_of"
            | "min"
            | "max"
            | "min_element"
            | "max_element"
            | "lower_bound"
            | "upper_bound"
            | "binary_search"
            | "unique"
            | "reverse"
            | "rotate"
            | "fill"
            | "generate"
            | "remove"
            | "remove_if"
            | "erase"
            | "swap"
            | "move"
            | "forward"
            | "begin"
            | "end"
            | "make_pair"
            | "make_tuple"
            | "get"
            | "tie"
            | "to_string"
            | "stoi"
            | "stol"
            | "stoul"
            | "stof"
            | "stod"
            // STL type traits / metaprogramming (stripped prefix)
            | "remove_const_t"
            | "remove_reference_t"
            | "decay_t"
            | "enable_if_t"
            | "conditional_t"
            | "is_same"
            | "is_same_v"
            | "is_integral"
            | "is_integral_v"
            | "is_floating_point"
            | "is_floating_point_v"
            | "is_pointer"
            | "is_pointer_v"
            | "is_reference"
            | "is_reference_v"
            | "is_const"
            | "is_const_v"
            | "is_base_of"
            | "is_base_of_v"
            | "is_convertible"
            | "is_convertible_v"
            | "numeric_limits"
            | "char_traits"
            | "allocator_traits"
            | "iterator_traits"
            // C fundamental types / POSIX typedefs
            | "size_t"
            | "ptrdiff_t"
            | "intptr_t"
            | "uintptr_t"
            | "int8_t"
            | "int16_t"
            | "int32_t"
            | "int64_t"
            | "uint8_t"
            | "uint16_t"
            | "uint32_t"
            | "uint64_t"
            | "int_fast8_t"
            | "int_fast16_t"
            | "int_fast32_t"
            | "int_fast64_t"
            | "intmax_t"
            | "uintmax_t"
            | "FILE"
            | "errno"
            | "NULL"
            | "EOF"
            | "INFINITY"
            | "NAN"
            | "INT_MAX"
            | "INT_MIN"
            | "UINT_MAX"
            | "LONG_MAX"
            | "LONG_MIN"
            | "ULONG_MAX"
            | "SIZE_MAX"
            // GTest / GMock macros and assertion functions
            | "ASSERT_EQ"
            | "ASSERT_NE"
            | "ASSERT_TRUE"
            | "ASSERT_FALSE"
            | "ASSERT_LT"
            | "ASSERT_LE"
            | "ASSERT_GT"
            | "ASSERT_GE"
            | "ASSERT_STREQ"
            | "ASSERT_STRNE"
            | "ASSERT_FLOAT_EQ"
            | "ASSERT_DOUBLE_EQ"
            | "ASSERT_THROW"
            | "ASSERT_NO_THROW"
            | "EXPECT_EQ"
            | "EXPECT_NE"
            | "EXPECT_TRUE"
            | "EXPECT_FALSE"
            | "EXPECT_LT"
            | "EXPECT_LE"
            | "EXPECT_GT"
            | "EXPECT_GE"
            | "EXPECT_STREQ"
            | "EXPECT_STRNE"
            | "EXPECT_THROW"
            | "EXPECT_NO_THROW"
            | "EXPECT_CALL"
            | "TEST"
            | "TEST_F"
            | "TEST_P"
            | "TYPED_TEST"
            | "INSTANTIATE_TEST_SUITE_P"
            | "MOCK_METHOD"
            | "ON_CALL"
            | "Return"
            | "ReturnRef"
            | "SetArgPointee"
            | "InvokeArgument"
    )
}
