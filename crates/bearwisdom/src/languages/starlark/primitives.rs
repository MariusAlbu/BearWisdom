// =============================================================================
// starlark/primitives.rs — Starlark/Bazel primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Starlark/Bazel.
pub(crate) const PRIMITIVES: &[&str] = &[
    // core builtins
    "fail", "print",
    "True", "False", "None",
    "len", "range", "enumerate", "zip",
    "sorted", "reversed",
    "list", "dict", "tuple", "set",
    "str", "int", "float", "bool",
    "type", "repr", "hash", "dir",
    "getattr", "hasattr",
    "any", "all", "min", "max", "abs",
    "struct", "provider", "depset", "select",
    // BUILD file builtins
    "glob", "package", "exports_files", "licenses",
    "load", "rule", "aspect",
    "attr", "repository_rule", "module_extension", "tag_class",
    "Label", "native", "ctx",
    "actions",
    // providers / info objects
    "DefaultInfo", "OutputGroupInfo", "RunEnvironmentInfo",
    "InstrumentedFilesInfo", "CcInfo", "JavaInfo",
    "ProguardSpecProvider", "PyInfo",
    // native rules
    "cc_binary", "cc_library", "cc_test",
    "java_binary", "java_library", "java_test",
    "py_binary", "py_library", "py_test",
    "sh_binary", "sh_library", "sh_test",
    "genrule", "filegroup", "test_suite",
    "config_setting", "alias",
    "platform", "constraint_setting", "constraint_value",
    "toolchain", "toolchain_type",
    "visibility",
    // Skylib / analysistest
    "asserts.equals", "asserts.true", "asserts.false",
    "asserts.new_set_equals",
    "analysistest.begin", "analysistest.end",
    "analysistest.target_under_test",
    "unittest.suite", "unittest.begin", "unittest.end", "unittest.fail",
    "echo",
];
