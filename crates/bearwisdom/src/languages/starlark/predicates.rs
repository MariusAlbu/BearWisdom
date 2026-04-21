// =============================================================================
// starlark/predicates.rs — Bazel / Starlark builtin functions and rules
// =============================================================================

use crate::types::EdgeKind;

/// Edge-kind / symbol-kind compatibility for Starlark.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "function" | "method" | "variable"),
        _ => true,
    }
}

/// Bazel framework parameter roots whose dotted-attribute chains are all
/// external by definition. Covers refs of any depth:
///
/// - `ctx.actions.run_shell`        → root "ctx"
/// - `ctx.label.name`               → root "ctx"
/// - `repository_ctx.execute`       → root "repository_ctx"
/// - `env.expect.that_str`          → root "env"   (analysistest)
/// - `directory.glob`               → root "directory" (rules_distdir)
///
/// A dotted ref matches when its first segment equals one of these roots.
const BAZEL_FRAMEWORK_ROOTS: &[&str] = &[
    "ctx",
    "repository_ctx",
    "module_ctx",
    "env",
    "directory",
];

/// Return true when `name` is a dotted ref whose leading segment is a known
/// Bazel framework parameter root (see `BAZEL_FRAMEWORK_ROOTS`).
///
/// Used by both `is_starlark_builtin` and `infer_external_namespace` to catch
/// refs like `ctx.label.name`, `env.expect.that_str`, `directory.glob` that
/// are more than two levels deep and cannot be enumerated statically.
pub(super) fn is_bazel_framework_chain(name: &str) -> bool {
    let root = name.split('.').next().unwrap_or(name);
    BAZEL_FRAMEWORK_ROOTS.contains(&root)
}

/// Bazel native rules, Starlark built-in functions, and `native.*` helpers.
pub(super) fn is_starlark_builtin(name: &str) -> bool {
    // All `native.*` attribute calls (native.cc_library, native.glob, etc.)
    // are Bazel built-ins regardless of the specific method name.
    if name == "native" || name.starts_with("native.") {
        return true;
    }

    // Framework parameter chains at any depth.
    if is_bazel_framework_chain(name) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framework_chain_catches_ctx_at_any_depth() {
        assert!(is_bazel_framework_chain("ctx"), "bare ctx");
        assert!(is_bazel_framework_chain("ctx.actions"), "ctx.actions");
        assert!(is_bazel_framework_chain("ctx.actions.run_shell"), "ctx.actions.run_shell");
        assert!(is_bazel_framework_chain("ctx.label.name"), "ctx.label.name 3-level");
        assert!(is_bazel_framework_chain("ctx.label.workspace_name"), "ctx.label.workspace_name");
    }

    #[test]
    fn framework_chain_catches_repository_ctx() {
        assert!(is_bazel_framework_chain("repository_ctx"), "bare");
        assert!(is_bazel_framework_chain("repository_ctx.execute"), "method");
        assert!(is_bazel_framework_chain("repository_ctx.os.name"), "3-level");
    }

    #[test]
    fn framework_chain_catches_env_and_directory() {
        assert!(is_bazel_framework_chain("env"), "bare env");
        assert!(is_bazel_framework_chain("env.expect"), "env.expect");
        assert!(is_bazel_framework_chain("env.expect.that_str"), "3-level analysistest");
        assert!(is_bazel_framework_chain("env.expect.that_str.equals"), "4-level");
        assert!(is_bazel_framework_chain("directory.glob"), "directory.glob");
    }

    #[test]
    fn framework_chain_does_not_match_non_framework() {
        assert!(!is_bazel_framework_chain("cc_library"), "native rule");
        assert!(!is_bazel_framework_chain("paths.join"), "skylib helper");
        assert!(!is_bazel_framework_chain("my_func"), "user func");
        assert!(!is_bazel_framework_chain("native.cc_library"), "native.* (separate check)");
    }

    #[test]
    fn is_starlark_builtin_includes_framework_chains() {
        assert!(is_starlark_builtin("ctx.actions.run_shell"), "via framework chain");
        assert!(is_starlark_builtin("ctx.label.name"), "3-level via framework chain");
        assert!(is_starlark_builtin("env.expect.that_str"), "analysistest chain");
        assert!(is_starlark_builtin("repository_ctx.execute"), "repo_ctx method");
    }

    #[test]
    fn is_starlark_builtin_still_catches_enumerated_names() {
        assert!(is_starlark_builtin("cc_library"));
        assert!(is_starlark_builtin("genrule"));
        assert!(is_starlark_builtin("rule"));
        assert!(is_starlark_builtin("provider"));
        assert!(is_starlark_builtin("select"));
    }

    #[test]
    fn native_prefix_is_still_caught() {
        assert!(is_starlark_builtin("native.cc_library"));
        assert!(is_starlark_builtin("native.glob"));
        assert!(is_starlark_builtin("native"));
    }
}
