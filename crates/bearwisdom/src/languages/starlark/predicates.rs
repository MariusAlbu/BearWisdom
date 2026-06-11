// =============================================================================
// starlark/predicates.rs — Bazel / Starlark builtin functions and rules
// =============================================================================

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

/// Starlark spec global functions — the closed set of built-ins the Starlark
/// interpreter and Bazel's universal global namespace inject into every file.
/// These are language/build-spec constructs, never user-defined or library
/// symbols, so a bare reference to one declines before the ladder rather than
/// binding to a same-named project symbol. Bazel native RULES (`cc_library`,
/// `proto_library`, …) and skylib helpers are NOT here — they are provided by
/// loaded `.bzl` files / native rule sets, resolved as externals.
const STARLARK_SPEC_GLOBALS: &[&str] = &[
    "rule",
    "aspect",
    "provider",
    "depset",
    "struct",
    "select",
    "glob",
    "load",
    "attr",
    "repository_rule",
    "module_extension",
    "tag_class",
    "use_extension",
    "use_repo",
    "visibility",
];

/// True when `name` is one of the Starlark spec global built-in functions
/// (see `STARLARK_SPEC_GLOBALS`). Drives the profile's `builtin_skip`.
pub(super) fn is_starlark_spec_global(name: &str) -> bool {
    STARLARK_SPEC_GLOBALS.contains(&name)
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
#[path = "predicates_tests.rs"]
mod tests;
