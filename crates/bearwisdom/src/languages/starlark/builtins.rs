// =============================================================================
// starlark/builtins.rs — Bazel / Starlark builtin functions and rules
// =============================================================================

use crate::types::EdgeKind;

/// Edge-kind / symbol-kind compatibility for Starlark.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "function" | "method" | "variable"),
        _ => true,
    }
}

/// Bazel native rules, Starlark built-in functions, and `native.*` helpers.
pub(super) fn is_starlark_builtin(name: &str) -> bool {
    // All `native.*` attribute calls (native.cc_library, native.glob, etc.)
    // are Bazel built-ins regardless of the specific method name.
    if name == "native" || name.starts_with("native.") {
        return true;
    }

    matches!(
        name,
        // -----------------------------------------------------------------------
        // Starlark core language built-ins
        // -----------------------------------------------------------------------
        "rule"
            | "provider"
            | "aspect"
            | "repository_rule"
            | "module_extension"
            | "tag_class"
            | "attr"
            | "struct"
            | "Label"
            | "select"
            | "glob"
            | "package"
            | "package_group"
            | "exports_files"
            | "licenses"
            | "workspace"
            | "register_toolchains"
            | "register_execution_platforms"
            | "visibility"
            | "fail"
            | "print"
            | "type"
            | "str"
            | "repr"
            | "bool"
            | "int"
            | "float"
            | "list"
            | "tuple"
            | "dict"
            | "set"
            | "range"
            | "enumerate"
            | "zip"
            | "sorted"
            | "reversed"
            | "len"
            | "min"
            | "max"
            | "all"
            | "any"
            | "hasattr"
            | "getattr"
            | "setattr"
            | "delattr"
            | "dir"
            | "hash"
            | "id"
            | "load"
            | "depset"
            | "bind"
            | "use_extension"
            | "use_repo"
            // -----------------------------------------------------------------------
            // Bazel native BUILD rules
            // -----------------------------------------------------------------------
            | "cc_binary"
            | "cc_library"
            | "cc_test"
            | "cc_import"
            | "cc_proto_library"
            | "cc_shared_library"
            | "java_binary"
            | "java_library"
            | "java_test"
            | "java_import"
            | "java_proto_library"
            | "py_binary"
            | "py_library"
            | "py_test"
            | "py_runtime"
            | "go_binary"
            | "go_library"
            | "go_test"
            | "rust_binary"
            | "rust_library"
            | "rust_test"
            | "rust_proc_macro"
            | "sh_binary"
            | "sh_library"
            | "sh_test"
            | "genrule"
            | "filegroup"
            | "test_suite"
            | "config_setting"
            | "alias"
            | "toolchain"
            | "toolchain_type"
            | "platform"
            | "constraint_setting"
            | "constraint_value"
            | "proto_library"
            | "proto_lang_toolchain"
            | "distribs"
            // -----------------------------------------------------------------------
            // attr.* constructors (Bazel rule attribute declarations)
            // -----------------------------------------------------------------------
            | "attr.string"
            | "attr.label"
            | "attr.bool"
            | "attr.int"
            | "attr.string_list"
            | "attr.label_list"
            | "attr.output"
            | "attr.output_list"
            | "attr.string_dict"
            | "attr.label_keyed_string_dict"
            // -----------------------------------------------------------------------
            // ctx.* API (rule implementation context)
            // -----------------------------------------------------------------------
            | "ctx.actions.run"
            | "ctx.actions.run_shell"
            | "ctx.actions.declare_file"
            | "ctx.actions.declare_directory"
            | "ctx.actions.write"
            | "ctx.actions.expand_template"
            | "ctx.actions.symlink"
            | "ctx.actions.args"
            | "ctx.attr"
            | "ctx.label"
            | "ctx.file"
            | "ctx.files"
            | "ctx.outputs"
            | "ctx.executable"
            | "ctx.runfiles"
            | "ctx.workspace_name"
            | "ctx.configuration"
            | "ctx.bin_dir"
            | "ctx.genfiles_dir"
            | "ctx.var"
            // -----------------------------------------------------------------------
            // Skylib test utilities
            // -----------------------------------------------------------------------
            | "asserts.equals"
            | "asserts.true"
            | "asserts.false"
            | "asserts.set_equals"
            | "asserts.new_set_equals"
            | "unittest.make"
            | "unittest.begin"
            | "unittest.end"
            | "unittest.suite"
            | "analysistest.make"
            | "analysistest.begin"
            | "analysistest.end"
            | "sets.make"
            | "sets.is_equal"
            | "sets.union"
            | "sets.intersection"
            | "sets.difference"
            | "sets.contains"
            | "paths.join"
            | "paths.split_extension"
            | "paths.basename"
            | "paths.dirname"
            | "paths.is_normalized"
            | "paths.normalize"
            | "paths.relativize"
            | "paths.is_absolute"
            // -----------------------------------------------------------------------
            // Common Bazel utilities
            // -----------------------------------------------------------------------
            | "rlocation"
            | "config_common.toolchain_type"
            | "DefaultInfo"
            | "OutputGroupInfo"
            | "InstrumentedFilesInfo"
            | "CcInfo"
            | "JavaInfo"
            | "PyInfo"
            | "ProtoInfo"
            | "RunEnvironmentInfo"
            | "testing.ExecutionInfo"
    )
}
