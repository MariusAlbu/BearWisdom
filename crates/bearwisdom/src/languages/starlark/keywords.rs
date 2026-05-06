// =============================================================================
// starlark/keywords.rs — Starlark/Bazel primitive and built-in types
//
// Names that the Starlark interpreter treats as always-in-scope built-ins
// plus Bazel native rules and skylib helpers that have no walkable source.
// Bazel framework parameter chains (ctx.*, repository_ctx.*, env.*, ...)
// and Python/Starlark built-in method tails (.append, .endswith, .pop,
// ...) are handled by shape predicates in starlark/predicates.rs since
// they're catch-all patterns rather than enumerable lists.
// =============================================================================

pub(crate) const KEYWORDS: &[&str] = &[
    // core builtins
    "fail", "print",
    "True", "False", "None",
    "len", "range", "enumerate", "zip",
    "sorted", "reversed",
    "list", "dict", "tuple", "set",
    "str", "int", "float", "bool",
    "type", "repr", "hash", "dir", "id",
    "getattr", "hasattr", "setattr", "delattr",
    "any", "all", "min", "max", "abs",
    "struct", "provider", "depset", "select",
    "load", "bind", "use_extension", "use_repo",
    // BUILD file builtins
    "glob", "package", "package_group", "exports_files", "licenses",
    "rule", "aspect",
    "attr", "repository_rule", "module_extension", "tag_class",
    "Label", "native", "ctx",
    "actions",
    "register_toolchains", "register_execution_platforms",
    "workspace", "distribs",
    "rlocation",
    // attr.* constructors
    "attr.string", "attr.label", "attr.bool", "attr.int",
    "attr.string_list", "attr.label_list",
    "attr.output", "attr.output_list",
    "attr.string_dict", "attr.label_keyed_string_dict",
    // ctx.* API
    "ctx.actions.run", "ctx.actions.run_shell",
    "ctx.actions.declare_file", "ctx.actions.declare_directory",
    "ctx.actions.write", "ctx.actions.expand_template",
    "ctx.actions.symlink", "ctx.actions.args",
    "ctx.attr", "ctx.label", "ctx.file", "ctx.files",
    "ctx.outputs", "ctx.executable", "ctx.runfiles",
    "ctx.workspace_name", "ctx.configuration",
    "ctx.bin_dir", "ctx.genfiles_dir", "ctx.var",
    // providers / info objects
    "DefaultInfo", "OutputGroupInfo", "RunEnvironmentInfo",
    "InstrumentedFilesInfo", "CcInfo", "JavaInfo",
    "ProguardSpecProvider", "PyInfo", "ProtoInfo",
    "testing.ExecutionInfo",
    "config_common.toolchain_type",
    // native rules
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
    "platform", "constraint_setting", "constraint_value",
    "toolchain", "toolchain_type",
    "proto_library", "proto_lang_toolchain",
    "visibility",
    // Skylib helpers
    "sets.make", "sets.is_equal", "sets.union",
    "sets.intersection", "sets.difference", "sets.contains",
    "paths.join", "paths.split_extension", "paths.basename",
    "paths.dirname", "paths.is_normalized", "paths.normalize",
    "paths.relativize", "paths.is_absolute",
    // analysistest / unittest framework
    "asserts.equals", "asserts.true", "asserts.false",
    "asserts.set_equals", "asserts.new_set_equals",
    "analysistest.make", "analysistest.begin", "analysistest.end",
    "analysistest.target_under_test",
    "unittest.make", "unittest.begin", "unittest.end",
    "unittest.suite", "unittest.fail",
    "echo",
];
