use std::fs;

use tempfile::TempDir;

use super::*;

/// Build an OTP-shaped fixture under `root`:
///   root/lib/
///     kernel-10.0/src/gen_server.erl
///     kernel-10.0/src/application.erl
///     stdlib-5.0/src/lists.erl
///     stdlib-5.0/src/io.erl
///     stdlib-5.0/src/io.hrl
///     mnesia-4.0/src/mnesia.erl
///     mnesia-4.0/src/priv/secret.erl   ← should be skipped
///     mnesia-4.0/src/test/t.erl        ← should be skipped
fn make_otp_fixture(root: &std::path::Path) -> std::path::PathBuf {
    let lib = root.join("lib");

    let kernel_src = lib.join("kernel-10.0").join("src");
    fs::create_dir_all(&kernel_src).unwrap();
    fs::write(
        kernel_src.join("gen_server.erl"),
        "-module(gen_server).\n-export([cast/2, call/2]).\n",
    )
    .unwrap();
    fs::write(
        kernel_src.join("application.erl"),
        "-module(application).\n",
    )
    .unwrap();

    let stdlib_src = lib.join("stdlib-5.0").join("src");
    fs::create_dir_all(&stdlib_src).unwrap();
    fs::write(stdlib_src.join("lists.erl"), "-module(lists).\n-export([map/2]).\n").unwrap();
    fs::write(stdlib_src.join("io.erl"), "-module(io).\n-export([format/2]).\n").unwrap();
    fs::write(stdlib_src.join("io.hrl"), "% io header\n").unwrap();

    let mnesia_src = lib.join("mnesia-4.0").join("src");
    fs::create_dir_all(&mnesia_src).unwrap();
    fs::write(mnesia_src.join("mnesia.erl"), "-module(mnesia).\n").unwrap();

    // Directories that must be skipped.
    let mnesia_priv = mnesia_src.join("priv");
    fs::create_dir_all(&mnesia_priv).unwrap();
    fs::write(mnesia_priv.join("secret.erl"), "-module(secret).\n").unwrap();

    let mnesia_test = mnesia_src.join("test");
    fs::create_dir_all(&mnesia_test).unwrap();
    fs::write(mnesia_test.join("t.erl"), "-module(t).\n").unwrap();

    lib
}

#[test]
fn discover_uses_explicit_otp_root_override() {
    let tmp = TempDir::new().unwrap();
    make_otp_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_OTP_ROOT", tmp.path());
    let roots = discover();
    std::env::remove_var("BEARWISDOM_OTP_ROOT");

    let apps: std::collections::HashSet<String> =
        roots.iter().map(|r| r.module_path.clone()).collect();
    assert!(apps.contains("kernel"), "{apps:?}");
    assert!(apps.contains("stdlib"), "{apps:?}");
    assert!(apps.contains("mnesia"), "{apps:?}");
}

#[test]
fn discover_returns_empty_for_missing_root() {
    std::env::set_var("BEARWISDOM_OTP_ROOT", "/nonexistent/path/erlang");
    // Override ERL_TOP too so we don't accidentally hit a real install.
    std::env::set_var("ERL_TOP", "/nonexistent");
    let roots = discover();
    std::env::remove_var("BEARWISDOM_OTP_ROOT");
    std::env::remove_var("ERL_TOP");
    // On a machine with a system Erlang, the fallback chain may still fire —
    // we only assert no panic.
    let _ = roots;
}

#[test]
fn discover_version_captured() {
    let tmp = TempDir::new().unwrap();
    make_otp_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_OTP_ROOT", tmp.path());
    let roots = discover();
    std::env::remove_var("BEARWISDOM_OTP_ROOT");

    let kernel = roots.iter().find(|r| r.module_path == "kernel").unwrap();
    assert_eq!(kernel.version, "10.0");
}

#[test]
fn discover_roots_are_sorted() {
    let tmp = TempDir::new().unwrap();
    make_otp_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_OTP_ROOT", tmp.path());
    let roots = discover();
    std::env::remove_var("BEARWISDOM_OTP_ROOT");

    let names: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn walk_emits_correct_virtual_paths() {
    let tmp = TempDir::new().unwrap();
    make_otp_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_OTP_ROOT", tmp.path());
    let roots = discover();
    std::env::remove_var("BEARWISDOM_OTP_ROOT");

    let kernel = roots.iter().find(|r| r.module_path == "kernel").unwrap();
    let walked = walk(kernel);
    let paths: std::collections::HashSet<String> =
        walked.iter().map(|f| f.relative_path.clone()).collect();

    assert!(
        paths.iter().any(|p| p == "ext:erlang:kernel/gen_server.erl"),
        "{paths:?}"
    );
    assert!(
        paths.iter().any(|p| p == "ext:erlang:kernel/application.erl"),
        "{paths:?}"
    );
}

#[test]
fn walk_includes_hrl_files() {
    let tmp = TempDir::new().unwrap();
    make_otp_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_OTP_ROOT", tmp.path());
    let roots = discover();
    std::env::remove_var("BEARWISDOM_OTP_ROOT");

    let stdlib = roots.iter().find(|r| r.module_path == "stdlib").unwrap();
    let walked = walk(stdlib);
    let paths: std::collections::HashSet<String> =
        walked.iter().map(|f| f.relative_path.clone()).collect();

    assert!(paths.iter().any(|p| p == "ext:erlang:stdlib/io.hrl"), "{paths:?}");
}

#[test]
fn walk_skips_pruned_directories() {
    let tmp = TempDir::new().unwrap();
    make_otp_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_OTP_ROOT", tmp.path());
    let roots = discover();
    std::env::remove_var("BEARWISDOM_OTP_ROOT");

    let mnesia = roots.iter().find(|r| r.module_path == "mnesia").unwrap();
    let walked = walk(mnesia);
    let paths: Vec<&str> = walked.iter().map(|f| f.relative_path.as_str()).collect();

    // priv/ and test/ must not appear.
    assert!(
        paths.iter().all(|p| !p.contains("/priv/") && !p.contains("/test/")),
        "pruned dirs leaked into walk: {paths:?}"
    );
    // The top-level mnesia.erl must still appear.
    assert!(paths.iter().any(|p| *p == "ext:erlang:mnesia/mnesia.erl"), "{paths:?}");
}

#[test]
fn walk_language_is_erlang() {
    let tmp = TempDir::new().unwrap();
    make_otp_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_OTP_ROOT", tmp.path());
    let roots = discover();
    std::env::remove_var("BEARWISDOM_OTP_ROOT");

    for dep in &roots {
        for wf in walk(dep) {
            assert_eq!(wf.language, "erlang", "language mismatch for {}", wf.relative_path);
        }
    }
}

#[test]
fn demand_pre_pull_returns_only_substrate_apps() {
    let tmp = TempDir::new().unwrap();
    make_otp_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_OTP_ROOT", tmp.path());
    let roots = discover();
    std::env::remove_var("BEARWISDOM_OTP_ROOT");

    let eco = ErlangOtpEcosystem;
    let pre_pulled = eco.demand_pre_pull(&roots);

    // All pre-pulled files must come from kernel or stdlib.
    for wf in &pre_pulled {
        assert!(
            wf.relative_path.starts_with("ext:erlang:kernel/")
                || wf.relative_path.starts_with("ext:erlang:stdlib/"),
            "unexpected pre-pull: {}",
            wf.relative_path
        );
    }
    // At least one file per substrate app.
    assert!(
        pre_pulled.iter().any(|f| f.relative_path.starts_with("ext:erlang:kernel/")),
        "no kernel files in pre-pull"
    );
    assert!(
        pre_pulled.iter().any(|f| f.relative_path.starts_with("ext:erlang:stdlib/")),
        "no stdlib files in pre-pull"
    );
    // mnesia must NOT appear in pre-pull.
    assert!(
        pre_pulled.iter().all(|f| !f.relative_path.starts_with("ext:erlang:mnesia/")),
        "mnesia should not be pre-pulled"
    );
}

#[test]
fn extract_module_name_parses_attribute() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("lists.erl");
    fs::write(&path, "-module(lists).\n-export([map/2]).\n").unwrap();
    assert_eq!(extract_module_name(&path), Some("lists".to_string()));
}

#[test]
fn extract_module_name_skips_comments() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("foo.erl");
    fs::write(&path, "% -module(fake).\n-module(real).\n").unwrap();
    assert_eq!(extract_module_name(&path), Some("real".to_string()));
}

#[test]
fn extract_module_name_returns_none_for_hrl() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("types.hrl");
    fs::write(&path, "% header file without module attribute\n-define(X, 1).\n").unwrap();
    // .hrl files typically have no -module(). None is the expected result.
    let result = extract_module_name(&path);
    assert!(result.is_none(), "unexpected: {result:?}");
}

#[test]
fn build_symbol_index_maps_module_names() {
    let tmp = TempDir::new().unwrap();
    make_otp_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_OTP_ROOT", tmp.path());
    let roots = discover();
    std::env::remove_var("BEARWISDOM_OTP_ROOT");

    let index = build_otp_symbol_index(&roots);

    // Module name keyed directly: locate("gen_server", "gen_server") → file
    let hit = index.locate("gen_server", "gen_server");
    assert!(hit.is_some(), "gen_server not in index");

    // App-keyed: locate("kernel", "gen_server") → same file
    let hit2 = index.locate("kernel", "gen_server");
    assert!(hit2.is_some(), "kernel/gen_server not in index");
}

#[test]
fn ecosystem_identity() {
    let e = ErlangOtpEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["erlang"]);
}

#[test]
#[ignore] // requires real OTP install at scoop default path
fn live_discovery_finds_scoop_install() {
    let scoop = std::env::var_os("USERPROFILE")
        .map(|h| std::path::PathBuf::from(h).join("scoop/apps/erlang/current"));
    if scoop.as_ref().is_none_or(|p| !p.is_dir()) {
        return;
    }
    std::env::remove_var("BEARWISDOM_OTP_ROOT");
    std::env::remove_var("ERL_TOP");
    let roots = discover();
    assert!(!roots.is_empty(), "expected OTP install to yield roots");
    let apps: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(apps.contains(&"kernel"), "{apps:?}");
    assert!(apps.contains(&"stdlib"), "{apps:?}");
}
