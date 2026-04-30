// Sibling test file for `bazel_central_registry.rs`.

use super::*;

#[test]
fn ecosystem_identity() {
    let eco = BazelCentralRegistryEcosystem;
    assert_eq!(eco.id(), ID);
    assert_eq!(Ecosystem::kind(&eco), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&eco), &["starlark"]);
    assert_eq!(eco.id().as_str(), "bazel-central-registry");
}

#[test]
fn parse_module_bazel_extracts_bazel_deps() {
    let content = r#"
module(
    name = "bazel_skylib",
    version = "1.9.0",
    compatibility_level = 1,
)

bazel_dep(name = "platforms", version = "0.0.10")
bazel_dep(name = "rules_license", version = "1.0.0")
bazel_dep(name = "stardoc", version = "0.8.0", dev_dependency = True, repo_name = "io_bazel_stardoc")
bazel_dep(name = "rules_cc", version = "0.0.17", dev_dependency = True)
"#;
    let deps = extract_bzlmod_deps(content);
    assert!(deps.contains(&"platforms".to_string()), "platforms missing");
    assert!(deps.contains(&"rules_license".to_string()), "rules_license missing");
    assert!(deps.contains(&"stardoc".to_string()), "stardoc missing");
    assert!(deps.contains(&"rules_cc".to_string()), "rules_cc missing");
    // module() itself is not a dep.
    assert!(!deps.contains(&"bazel_skylib".to_string()), "module name should not be a dep");
}

#[test]
fn parse_workspace_extracts_http_archive_deps() {
    let content = r#"
workspace(name = "bazel_skylib")

http_archive(
    name = "rules_cc",
    sha256 = "abc605dd850f813bb37004b77db20106a19311a96b2da1c92b789da529d28fe1",
    strip_prefix = "rules_cc-0.0.17",
    urls = ["https://github.com/bazelbuild/rules_cc/releases/download/0.0.17/rules_cc-0.0.17.tar.gz"],
)

http_archive(
    name = "rules_shell",
    sha256 = "d8cd4a3a91fc1dc68d4c7d6b655f09def109f7186437e3f50a9b60ab436a0c53",
    url = "https://github.com/bazelbuild/rules_shell/releases/download/v0.3.0/rules_shell-v0.3.0.tar.gz",
)
"#;
    let deps = extract_workspace_deps(content);
    assert!(deps.contains(&"rules_cc".to_string()), "rules_cc missing from WORKSPACE");
    assert!(deps.contains(&"rules_shell".to_string()), "rules_shell missing from WORKSPACE");
}

#[test]
fn builtin_rules_contains_cc_library() {
    let pf = synth_builtin_rules();
    assert_eq!(pf.path, "ext:bazel-builtins:rules.bzl");
    assert_eq!(pf.language, "starlark");
    let has_cc = pf.symbols.iter().any(|s| s.name == "cc_library");
    assert!(has_cc, "cc_library not in builtin rules");
    let has_genrule = pf.symbols.iter().any(|s| s.name == "genrule");
    assert!(has_genrule, "genrule not in builtin rules");
    assert_eq!(pf.symbols.len(), BUILTIN_RULES.len());
}

#[test]
fn builtin_rule_count() {
    // Keep in sync with the BUILTIN_RULES constant.
    assert_eq!(BUILTIN_RULES.len(), 23);
}

#[test]
fn walk_bazel_root_returns_starlark_files() {
    let tmp = std::env::temp_dir().join("bw-test-bazel-walk");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("lib")).unwrap();
    std::fs::write(tmp.join("lib").join("paths.bzl"), "def join(*args): pass").unwrap();
    std::fs::write(tmp.join("BUILD"), "filegroup(name = \"all\")").unwrap();
    std::fs::write(tmp.join("not_starlark.py"), "x = 1").unwrap();

    let dep = ExternalDepRoot {
        module_path: "test_dep".to_string(),
        version: String::new(),
        root: tmp.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let files = walk_bazel_root(&dep);
    assert_eq!(files.len(), 2, "expected BUILD + paths.bzl, got {}", files.len());
    assert!(files.iter().all(|f| f.language == "starlark"));
    assert!(files.iter().any(|f| f.relative_path.ends_with("paths.bzl")));
    assert!(files.iter().any(|f| f.relative_path.ends_with("BUILD")));
    // .py files must not appear.
    assert!(files.iter().all(|f| !f.relative_path.ends_with(".py")));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn legacy_locator_tag() {
    assert_eq!(
        ExternalSourceLocator::ecosystem(&BazelCentralRegistryEcosystem),
        "bazel-central-registry"
    );
}

#[allow(dead_code)]
fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
    shared_locator()
}

#[test]
fn synth_ctx_api_has_expected_symbols() {
    let pf = synth_ctx_api();
    assert_eq!(pf.path, "ext:bazel-builtins:ctx.bzl");
    assert_eq!(pf.language, "starlark");

    let has_run_shell = pf.symbols.iter().any(|s| s.qualified_name == "ctx.actions.run_shell");
    assert!(has_run_shell, "ctx.actions.run_shell not in ctx API");

    let has_label_name = pf.symbols.iter().any(|s| s.qualified_name == "ctx.label.name");
    assert!(has_label_name, "ctx.label.name not in ctx API");

    let has_label_pkg = pf.symbols.iter().any(|s| s.qualified_name == "ctx.label.package");
    assert!(has_label_pkg, "ctx.label.package not in ctx API");

    let has_repo_execute = pf.symbols.iter().any(|s| s.qualified_name == "repository_ctx.execute");
    assert!(has_repo_execute, "repository_ctx.execute not in ctx API");

    let has_repo_os = pf.symbols.iter().any(|s| s.qualified_name == "repository_ctx.os");
    assert!(has_repo_os, "repository_ctx.os not in ctx API");

    // synth_ctx_api emits CTX_MEMBERS, REPOSITORY_CTX_MEMBERS once,
    // a copy per MODULE_CTX_ALIASES (mctx/mrctx/module_ctx), then
    // TARGET/RUNFILES/ARGS members, ARGS_LOCAL_ALIASES expansions,
    // ATTR/CONFIG factories, PROVIDER/RESULT/TEST_RESULT members,
    // and TOP_LEVEL_BUILTIN_RULES. Recompute and assert once so the
    // test catches accidental drops without needing manual updates.
    let args_alias_total: usize = ARGS_LOCAL_ALIASES
        .iter()
        .map(|(_, methods)| methods.len())
        .sum();
    let expected_count = CTX_MEMBERS.len()
        + REPOSITORY_CTX_MEMBERS.len()
        + MODULE_CTX_ALIASES.len() * REPOSITORY_CTX_MEMBERS.len()
        + TARGET_MEMBERS.len()
        + RUNFILES_MEMBERS.len()
        + ARGS_MEMBERS.len()
        + args_alias_total
        + ATTR_FACTORIES.len()
        + CONFIG_FACTORIES.len()
        + PROVIDER_MEMBERS.len()
        + RESULT_TYPE_MEMBERS.len()
        + TEST_RESULT_MEMBERS.len()
        + TOP_LEVEL_BUILTIN_RULES.len();
    assert_eq!(
        pf.symbols.len(), expected_count,
        "ctx API symbol count mismatch: expected {expected_count}, got {}",
        pf.symbols.len()
    );
}

#[test]
fn parse_metadata_only_returns_all_synth_files() {
    let eco = BazelCentralRegistryEcosystem;
    let dep = ExternalDepRoot {
        module_path: "dummy".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("/tmp"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let files = <BazelCentralRegistryEcosystem as Ecosystem>::parse_metadata_only(&eco, &dep)
        .expect("expected Some");
    assert_eq!(files.len(), 3, "expected rules.bzl + ctx.bzl + env.bzl synthetic files");
    assert!(files.iter().any(|f| f.path == "ext:bazel-builtins:rules.bzl"));
    assert!(files.iter().any(|f| f.path == "ext:bazel-builtins:ctx.bzl"));
    assert!(files.iter().any(|f| f.path == "ext:bazel-builtins:env.bzl"));
}

#[test]
fn synth_env_api_has_expected_symbols() {
    let pf = synth_env_api();
    assert_eq!(pf.path, "ext:bazel-builtins:env.bzl");
    assert_eq!(pf.language, "starlark");

    // Top-level env members.
    assert!(pf.symbols.iter().any(|s| s.qualified_name == "env.expect"),
        "env.expect missing");
    assert!(pf.symbols.iter().any(|s| s.qualified_name == "env.fail"),
        "env.fail missing");
    assert!(pf.symbols.iter().any(|s| s.qualified_name == "env.assert_equals"),
        "env.assert_equals missing");

    // env_expect type-level factory methods.
    assert!(pf.symbols.iter().any(|s| s.qualified_name == "env_expect.that_str"),
        "env_expect.that_str missing");
    assert!(pf.symbols.iter().any(|s| s.qualified_name == "env_expect.that_collection"),
        "env_expect.that_collection missing");

    // Flat dotted aliases (what the chain walker looks up).
    assert!(pf.symbols.iter().any(|s| s.qualified_name == "env.expect.that_str"),
        "env.expect.that_str flat alias missing");
    assert!(pf.symbols.iter().any(|s| s.qualified_name == "env.expect.that_collection"),
        "env.expect.that_collection flat alias missing");

    // Subject assertion methods.
    assert!(pf.symbols.iter().any(|s| s.qualified_name == "env_str_subject.equals"),
        "env_str_subject.equals missing");
    assert!(pf.symbols.iter().any(|s| s.qualified_name == "env_collection_subject.contains"),
        "env_collection_subject.contains missing");
    assert!(pf.symbols.iter().any(|s| s.qualified_name == "env_bool_subject.is_true"),
        "env_bool_subject.is_true missing");

    // Total = ENV_MEMBERS + ENV_EXPECT_MEMBERS + ENV_EXPECT_FLAT_ALIASES +
    //         (subject types x assertion methods).
    let expected = ENV_MEMBERS.len()
        + ENV_EXPECT_MEMBERS.len()
        + ENV_EXPECT_FLAT_ALIASES.len()
        + ENV_SUBJECT_TYPES.len() * SUBJECT_ASSERTION_METHODS.len();
    assert_eq!(
        pf.symbols.len(), expected,
        "env API symbol count mismatch: expected {expected}, got {}",
        pf.symbols.len()
    );
}
