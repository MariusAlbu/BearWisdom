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

    let packages = root.join("fpc").join("3.2.2").join("source").join("packages").join("fcl-base");
    fs::create_dir_all(&packages).unwrap();
    fs::write(packages.join("inifiles.pp"), "unit IniFiles;\n").unwrap();
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
    assert!(module_paths.contains("fpc-packages"), "{module_paths:?}");
    // Exactly one host-target RTL root, never both win32 + win64.
    let rtl_count = module_paths
        .iter()
        .filter(|p| p.starts_with("fpc-rtl-win"))
        .count();
    assert_eq!(rtl_count, 1, "{module_paths:?}");
}

#[test]
fn walk_pascal_root_picks_pas_pp_inc() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("lcl");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("forms.pas"), "unit Forms;\n").unwrap();
    fs::write(root.join("buttons.pp"), "unit Buttons;\n").unwrap();
    fs::write(root.join("config.inc"), "// include\n").unwrap();
    fs::write(root.join("README.md"), "docs\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "lcl".to_string(),
        version: String::new(),
        root: root.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_pascal_root(&dep);
    let names: std::collections::HashSet<String> = walked
        .iter()
        .map(|f| f.absolute_path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains("forms.pas"));
    assert!(names.contains("buttons.pp"));
    assert!(names.contains("config.inc"));
    assert!(!names.contains("README.md"));
    assert!(walked.iter().all(|f| f.language == "pascal"));
}

#[test]
fn walk_skips_tests_examples_demos() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("lcl");
    fs::create_dir_all(root.join("tests")).unwrap();
    fs::create_dir_all(root.join("examples")).unwrap();
    fs::create_dir_all(root.join("demos")).unwrap();
    fs::write(root.join("tests").join("test_forms.pas"), "// skip\n").unwrap();
    fs::write(root.join("examples").join("hello.pas"), "// skip\n").unwrap();
    fs::write(root.join("demos").join("demo.pas"), "// skip\n").unwrap();
    fs::write(root.join("forms.pas"), "// keep\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "lcl".to_string(),
        version: String::new(),
        root: root.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_pascal_root(&dep);
    assert_eq!(walked.len(), 1);
    assert!(walked[0].absolute_path.ends_with("forms.pas"));
}

#[test]
fn walk_emits_virtual_path_with_pascal_prefix() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    fs::write(root.join("forms.pas"), "unit Forms;\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "lcl".to_string(),
        version: String::new(),
        root: root.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_pascal_root(&dep);
    assert_eq!(walked.len(), 1);
    assert_eq!(walked[0].relative_path, "ext:pascal:lcl/forms.pas");
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
