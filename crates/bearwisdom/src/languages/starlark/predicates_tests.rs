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
