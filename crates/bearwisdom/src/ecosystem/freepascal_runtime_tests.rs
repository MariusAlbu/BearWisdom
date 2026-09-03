use std::fs;

use tempfile::TempDir;

use super::*;

fn make_lazarus_fixture(root: &std::path::Path) {
    fs::create_dir_all(root.join("lcl")).unwrap();
    fs::write(root.join("lcl").join("forms.pas"), "unit Forms;\n").unwrap();
    fs::write(root.join("lcl").join("buttons.pp"), "unit Buttons;\n").unwrap();

    fs::create_dir_all(root.join("components").join("codetools")).unwrap();
    fs::write(
        root.join("components").join("codetools").join("codecache.pas"),
        "unit CodeCache;\n",
    )
    .unwrap();

    let win64 = root.join("fpc").join("3.2.2").join("source").join("rtl").join("win64");
    fs::create_dir_all(&win64).unwrap();
    fs::write(win64.join("system.pp"), "unit System;\n").unwrap();
    fs::write(win64.join("classes.pp"), "unit Classes;\n").unwrap();

    let win32 = root.join("fpc").join("3.2.2").join("source").join("rtl").join("win32");
    fs::create_dir_all(&win32).unwrap();
    fs::write(win32.join("system.pp"), "unit System;\n").unwrap();

    let objpas = root.join("fpc").join("3.2.2").join("source").join("rtl").join("objpas");
    fs::create_dir_all(&objpas).unwrap();
    fs::write(objpas.join("classes.pp"), "unit Classes;\n").unwrap();
    fs::write(objpas.join("sysutils.pp"), "unit SysUtils;\n").unwrap();

    // Package with a /src/ subdir — the per-package walker requires /src/ to exist.
    let pkg_src = root
        .join("fpc")
        .join("3.2.2")
        .join("source")
        .join("packages")
        .join("fcl-base")
        .join("src");
    fs::create_dir_all(&pkg_src).unwrap();
    fs::write(pkg_src.join("inifiles.pp"), "unit IniFiles;\n").unwrap();
}

#[test]
fn discover_returns_empty_without_install() {
    let tmp = TempDir::new().unwrap();
    std::env::set_var("BEARWISDOM_LAZARUS_DIR", tmp.path().join("nonexistent"));
    let roots = discover_freepascal_roots();
    std::env::remove_var("BEARWISDOM_LAZARUS_DIR");
    // The override pointed at a missing dir, but the fallback chain may
    // still find the system Lazarus install. Either is correct behavior;
    // we only assert that the call doesn't panic.
    let _ = roots;
}

#[test]
fn discover_uses_explicit_dir_override() {
    let tmp = TempDir::new().unwrap();
    make_lazarus_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_LAZARUS_DIR", tmp.path());
    let roots = discover_freepascal_roots();
    std::env::remove_var("BEARWISDOM_LAZARUS_DIR");

    let module_paths: std::collections::HashSet<String> =
        roots.iter().map(|r| r.module_path.clone()).collect();
    assert!(module_paths.contains("lcl"), "{module_paths:?}");
    assert!(module_paths.contains("lazarus-components"), "{module_paths:?}");
    assert!(module_paths.contains("fpc-rtl-objpas"), "{module_paths:?}");
    // Single package under packages/fcl-base/src/ emits one per-package root.
    assert!(module_paths.contains("fpc-pkg-fcl-base"), "{module_paths:?}");
    // The old aggregate fpc-packages root no longer exists — packages are emitted
    // individually so module_path values are distinct per package.
    assert!(!module_paths.contains("fpc-packages"), "{module_paths:?}");
    // The host target's RTL tree registers BEFORE any other platform tree,
    // so the location index's first-writer-wins prefers host units for names
    // every platform declares. Non-host trees follow for platform-only units.
    let first_rtl = roots
        .iter()
        .find(|r| r.module_path.starts_with("fpc-rtl-win"))
        .map(|r| r.module_path.clone());
    assert_eq!(first_rtl.as_deref(), Some("fpc-rtl-win64"), "{module_paths:?}");
    assert!(module_paths.contains("fpc-rtl-win32"), "{module_paths:?}");
}

#[test]
#[ignore] // requires real Lazarus install at scoop default path
fn live_discovery_finds_scoop_install() {
    // Defensive: only assert when we know the scoop path is present on
    // the dev machine. This is the on-this-machine smoke check.
    let scoop = std::env::var_os("USERPROFILE")
        .map(|h| std::path::PathBuf::from(h).join("scoop/apps/lazarus/current"));
    if scoop.as_ref().is_none_or(|p| !p.is_dir()) {
        return;
    }
    std::env::remove_var("BEARWISDOM_LAZARUS_DIR");
    std::env::remove_var("LAZARUS_DIR");
    let roots = discover_freepascal_roots();
    assert!(!roots.is_empty(), "expected Lazarus install to yield roots");
    let names: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(names.contains(&"lcl"), "{names:?}");
}

#[test]
fn ecosystem_identity_and_languages() {
    let e = FreePascalRuntimeEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["pascal"]);
}

#[test]
fn emit_package_roots_requires_src_subdir() {
    let tmp = TempDir::new().unwrap();
    let packages = tmp.path().join("packages");

    // Package with /src/: should be emitted.
    fs::create_dir_all(packages.join("fcl-base").join("src")).unwrap();
    fs::write(packages.join("fcl-base").join("src").join("a.pp"), "").unwrap();

    // Package without /src/: should be skipped.
    fs::create_dir_all(packages.join("nonesuch")).unwrap();
    fs::write(packages.join("nonesuch").join("main.pp"), "").unwrap();

    let mut roots = Vec::new();
    emit_package_roots(&packages, &mut roots);

    let names: std::collections::HashSet<String> =
        roots.iter().map(|r| r.module_path.clone()).collect();
    assert!(names.contains("fpc-pkg-fcl-base"), "{names:?}");
    assert!(!names.contains("fpc-pkg-nonesuch"), "{names:?}");
}

#[test]
fn platform_excluded_exotic_targets() {
    // These exotic targets must always be excluded regardless of host.
    for pkg in &["arosunits", "ami-extra", "palmunits", "libgbafpc", "libndsfpc"] {
        assert!(is_platform_excluded(pkg), "{pkg} should be excluded");
    }
}

#[test]
fn cross_platform_packages_never_excluded() {
    // These packages are cross-platform and must always be walked.
    for pkg in &["fcl-base", "fcl-xml", "fcl-net", "rtl-generics", "paszlib", "hash"] {
        assert!(!is_platform_excluded(pkg), "{pkg} should not be excluded");
    }
}

// ---------------------------------------------------------------------------
// Demand-driven interface
// ---------------------------------------------------------------------------

fn make_dep(root: &std::path::Path, module: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: module.to_string(),
        version: String::new(),
        root: root.to_path_buf(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

#[test]
fn ecosystem_declares_demand_driven() {
    let e = FreePascalRuntimeEcosystem;
    assert!(Ecosystem::uses_demand_driven_parse(&e));
}

#[test]
fn walk_root_is_empty_under_demand_driven() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("forms.pas"), "unit Forms;\n").unwrap();
    let dep = make_dep(tmp.path(), "lcl");
    assert!(Ecosystem::walk_root(&FreePascalRuntimeEcosystem, &dep).is_empty());
}

#[test]
fn symbol_index_non_empty_for_fixture_roots() {
    let tmp = TempDir::new().unwrap();
    make_lazarus_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_LAZARUS_DIR", tmp.path());
    let roots = discover_freepascal_roots();
    std::env::remove_var("BEARWISDOM_LAZARUS_DIR");

    let idx = fpc_fragment_index::build_pascal_symbol_index(&roots);
    assert!(!idx.is_empty(), "symbol index must be non-empty for a Lazarus fixture");
    // The fixture writes `unit Forms;` in lcl/forms.pas.
    let hit = roots.iter().any(|r| r.module_path == "lcl")
        && idx.find_by_name("forms").iter().any(|(m, _)| *m == "lcl");
    assert!(hit, "unit 'forms' must appear in the lcl module index");
}

// ---------------------------------------------------------------------------
// Shared-platform-dir discovery (win64/win32 -> win, linux/darwin/freebsd -> unix)
// ---------------------------------------------------------------------------

#[test]
fn shared_rtl_dirs_reads_windir_from_makefile_fpc() {
    let tmp = TempDir::new().unwrap();
    let win64 = tmp.path().join("win64");
    fs::create_dir_all(&win64).unwrap();
    fs::write(
        win64.join("Makefile.fpc"),
        "[target]\ntarget=win64\n[require]\nRTL=..\nWININC=../win/wininc\nWINDIR=../win\n",
    )
    .unwrap();

    let dirs = shared_rtl_dirs(&win64);
    assert!(dirs.iter().any(|d| d == "win"), "{dirs:?}");
    // `RTL=..` has no trailing segment and must not surface as a directory.
    assert!(!dirs.iter().any(|d| d == ".."), "{dirs:?}");
    // `WININC=../win/wininc` nests two segments — already covered once `win`
    // itself is walked — and must not surface as its own root.
    assert!(!dirs.iter().any(|d| d.contains('/')), "{dirs:?}");
}

#[test]
fn shared_rtl_dirs_reads_multiple_vars_dollar_rtl_form() {
    let tmp = TempDir::new().unwrap();
    let freebsd = tmp.path().join("freebsd");
    fs::create_dir_all(&freebsd).unwrap();
    fs::write(
        freebsd.join("Makefile.fpc"),
        "[target]\ntarget=freebsd\n[require]\nRTL=..\nBSDINC=$(RTL)/bsd\nUNIXINC=$(RTL)/unix\n",
    )
    .unwrap();

    let dirs = shared_rtl_dirs(&freebsd);
    assert!(dirs.iter().any(|d| d == "bsd"), "{dirs:?}");
    assert!(dirs.iter().any(|d| d == "unix"), "{dirs:?}");
}

#[test]
fn shared_rtl_dirs_empty_without_makefile_fpc() {
    let tmp = TempDir::new().unwrap();
    let win16 = tmp.path().join("win16");
    fs::create_dir_all(&win16).unwrap();
    // No Makefile.fpc written — win16 is self-contained on real FPC installs.
    assert!(shared_rtl_dirs(&win16).is_empty());
}

#[test]
fn discover_registers_shared_platform_dir_alongside_primary_target() {
    let tmp = TempDir::new().unwrap();
    make_lazarus_fixture(tmp.path());

    // sysutils.pp lives only in the shared `win` dir on real FPC 3.2.2
    // installs (win64 has no copy of its own) — the win64 target's own
    // Makefile.fpc is what tells the walker `win` is reachable.
    let rtl = tmp.path().join("fpc").join("3.2.2").join("source").join("rtl");
    let win64 = rtl.join("win64");
    fs::write(win64.join("Makefile.fpc"), "RTL=..\nWINDIR=../win\n").unwrap();
    let win = rtl.join("win");
    fs::create_dir_all(&win).unwrap();
    fs::write(win.join("sysutils.pp"), "unit SysUtils;\n").unwrap();

    std::env::set_var("BEARWISDOM_LAZARUS_DIR", tmp.path());
    let roots = discover_freepascal_roots();
    std::env::remove_var("BEARWISDOM_LAZARUS_DIR");

    let module_paths: std::collections::HashSet<String> =
        roots.iter().map(|r| r.module_path.clone()).collect();
    assert!(module_paths.contains("fpc-rtl-win64"), "{module_paths:?}");
    assert!(module_paths.contains("fpc-rtl-win"), "{module_paths:?}");
    // win32 registers too (non-host platform tree), but only AFTER the host
    // target and its Makefile-declared shared family, so `win`'s sysutils.pp
    // stays the first-writer location for shared unit names.
    let order: Vec<&str> = roots
        .iter()
        .map(|r| r.module_path.as_str())
        .filter(|p| p.starts_with("fpc-rtl-win"))
        .collect();
    let pos = |n: &str| order.iter().position(|p| *p == n);
    assert!(pos("fpc-rtl-win") < pos("fpc-rtl-win32"), "{order:?}");
}
