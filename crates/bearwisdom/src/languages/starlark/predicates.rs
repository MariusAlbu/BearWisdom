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
    // Common short-names used by rule_repository implementations
    "rctx",
    "mctx",
    // unittest / analysistest framework parameters
    "asserts",
    "actions",
    // Python convention parameter names that have no source-code
    // declaration: every Starlark function with `**kwargs` / `*args`
    // gets these as runtime-injected dict/tuple parameters.
    "kwargs",
    "attrs",
];

/// Return true when `name` is a dotted ref whose leading segment is a known
/// Bazel framework parameter root (see `BAZEL_FRAMEWORK_ROOTS`).
///
/// Used by `infer_external_namespace` to catch refs like `ctx.label.name`,
/// `env.expect.that_str`, `directory.glob` that are more than two levels
/// deep and cannot be enumerated statically. The engine's keywords() set
/// covers the enumerable surface (cc_library, paths.join, asserts.equals,
/// ...).
pub(super) fn is_bazel_framework_chain(name: &str) -> bool {
    let root = name.split('.').next().unwrap_or(name);
    BAZEL_FRAMEWORK_ROOTS.contains(&root)
}

/// True when a dotted call's last segment is a Python/Starlark built-in
/// type method (str/list/dict/set/depset). `auth_info.get`, `output.append`,
/// `filename.endswith`, `kwargs.pop` all bind to runtime types, never to
/// user-defined symbols.
pub(super) fn is_builtin_method_tail(name: &str) -> bool {
    let tail = name.rsplit('.').next().unwrap_or(name);
    matches!(
        tail,
        // dict
        "get" | "items" | "keys" | "values" | "pop" | "popitem" | "setdefault"
        | "update" | "clear" | "copy" | "fromkeys"
        // list
        | "append" | "extend" | "insert" | "remove" | "index" | "count"
        | "reverse" | "sort"
        // str
        | "split" | "rsplit" | "splitlines" | "join" | "format" | "format_map"
        | "startswith" | "endswith" | "find" | "rfind" | "replace"
        | "lower" | "upper" | "title" | "capitalize" | "swapcase"
        | "lstrip" | "rstrip" | "strip" | "isdigit" | "isalpha" | "isalnum"
        | "isspace" | "isupper" | "islower" | "isnumeric" | "isdecimal"
        | "encode" | "decode" | "elems" | "codepoints"
        | "removeprefix" | "removesuffix"
        | "partition" | "rpartition"
        // set / general
        | "add" | "discard" | "intersection" | "union" | "difference"
        // depset
        | "to_list" | "to_set"
        // common iterators
        | "next"
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

    // Tests for the deleted is_starlark_builtin function were removed.
    // Bazel native rules and Skylib helpers now classify via the
    // engine's keywords() set populated from starlark/keywords.rs;
    // framework parameter chains classify via is_bazel_framework_chain
    // (still tested above).
}
