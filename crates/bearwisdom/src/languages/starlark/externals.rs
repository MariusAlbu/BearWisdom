/// Bazel / Starlark built-in functions, native rules, and `native.*` helpers —
/// always external (never defined inside a project).
pub(crate) const EXTERNALS: &[&str] = &[
    // -------------------------------------------------------------------------
    // Starlark language built-ins
    // -------------------------------------------------------------------------
    "rule", "provider", "aspect", "repository_rule", "module_extension",
    "tag_class", "attr", "struct", "Label", "select", "glob",
    "package", "package_group", "exports_files", "licenses",
    "workspace", "register_toolchains", "register_execution_platforms",
    "visibility", "fail", "print", "type", "str", "repr",
    "bool", "int", "float", "list", "tuple", "dict", "set",
    "range", "enumerate", "zip", "sorted", "reversed",
    "len", "min", "max", "all", "any",
    "hasattr", "getattr", "setattr", "delattr",
    "dir", "hash", "id", "load",
    "depset", "bind", "use_extension", "use_repo",
    // -------------------------------------------------------------------------
    // Bazel native BUILD rules
    // -------------------------------------------------------------------------
    "cc_binary", "cc_library", "cc_test", "cc_import",
    "cc_proto_library", "cc_shared_library",
    "java_binary", "java_library", "java_test", "java_import",
    "java_proto_library",
    "py_binary", "py_library", "py_test", "py_runtime",
    "go_binary", "go_library", "go_test",
    "rust_binary", "rust_library", "rust_test", "rust_proc_macro",
    "sh_binary", "sh_library", "sh_test",
    "genrule", "filegroup", "test_suite",
    "config_setting", "alias",
    "toolchain", "toolchain_type", "platform",
    "constraint_setting", "constraint_value",
    "proto_library", "proto_lang_toolchain",
    "distribs",
    // -------------------------------------------------------------------------
    // native.* namespace — common entries for the static bare-name fallback.
    // The resolver's infer_external_namespace handles the full `native.*`
    // prefix at runtime, so this list is supplementary coverage only.
    // -------------------------------------------------------------------------
    "native",
    "native.cc_binary",
    "native.cc_library",
    "native.cc_test",
    "native.java_binary",
    "native.java_library",
    "native.java_test",
    "native.py_binary",
    "native.py_library",
    "native.py_test",
    "native.genrule",
    "native.filegroup",
    "native.glob",
    "native.existing_rules",
    "native.package_name",
    "native.repository_name",
    "native.exports_files",
    "native.alias",
    "native.config_setting",
    "native.test_suite",
];
