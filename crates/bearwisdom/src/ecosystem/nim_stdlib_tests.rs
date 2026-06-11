use super::*;
use std::fs;

/// Lay down a minimal Nim lib fixture: `system.nim` is the marker; pure/ and
/// core/ hold importable modules; deprecated/ is noise.
fn make_lib_fixture(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("system.nim"), "proc echo*(x: string) = discard\n").unwrap();

    let pure = dir.join("pure");
    fs::create_dir_all(&pure).unwrap();
    fs::write(pure.join("strutils.nim"), "proc split*(s: string): seq[string] = @[]\n").unwrap();
    fs::write(pure.join("sequtils.nim"), "proc toSeq*(): seq[int] = @[]\n").unwrap();
    fs::write(pure.join("os.nim"), "proc getEnv*(k: string): string = \"\"\n").unwrap();

    let core = dir.join("core");
    fs::create_dir_all(&core).unwrap();
    fs::write(core.join("macros.nim"), "proc newLit*(): int = 0\n").unwrap();

    // Noise: deprecated/ modules must be pruned.
    let dep = dir.join("deprecated");
    fs::create_dir_all(&dep).unwrap();
    fs::write(dep.join("oldmod.nim"), "proc old*() = discard\n").unwrap();
}

#[test]
fn walk_yields_nim_files_pruning_deprecated() {
    let tmp = tempfile::tempdir().unwrap();
    make_lib_fixture(tmp.path());

    let dep = ExternalDepRoot {
        module_path: "stdlib".into(),
        version: String::new(),
        root: tmp.path().to_path_buf(),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let files = walk(&dep);
    let names: Vec<String> = files
        .iter()
        .map(|f| f.absolute_path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"system.nim".to_string()), "{names:?}");
    assert!(names.contains(&"strutils.nim".to_string()), "{names:?}");
    assert!(names.contains(&"macros.nim".to_string()), "{names:?}");
    assert!(
        !names.contains(&"oldmod.nim".to_string()),
        "deprecated/ modules must be pruned: {names:?}"
    );
    for f in &files {
        assert_eq!(f.language, "nim");
        assert!(f.relative_path.starts_with("ext:nim:"));
    }
}

#[test]
fn symbol_index_maps_bare_module_names() {
    let tmp = tempfile::tempdir().unwrap();
    make_lib_fixture(tmp.path());

    let dep = ExternalDepRoot {
        module_path: "stdlib".into(),
        version: String::new(),
        root: tmp.path().to_path_buf(),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let index = build_nim_symbol_index(&[dep]);

    // `import strutils` resolves by the bare stem under stdlib and by name.
    assert!(
        index.locate("stdlib", "strutils").is_some(),
        "strutils not indexed; {} entries",
        index.len()
    );
    assert!(index.locate("stdlib", "sequtils").is_some());
    assert!(index.locate("stdlib", "system").is_some());
    assert!(index.locate("stdlib", "macros").is_some());
}

#[test]
fn ascend_recovers_lib_root_from_subdir() {
    let tmp = tempfile::tempdir().unwrap();
    make_lib_fixture(tmp.path());
    // Starting from lib/pure, ascend finds lib/ (the dir with system.nim).
    let from_pure = tmp.path().join("pure");
    let root = ascend_to_lib_root(&from_pure).expect("lib root from pure/");
    assert_eq!(root, tmp.path());
}

#[test]
fn ascend_returns_none_for_unrelated_dir() {
    let tmp = tempfile::tempdir().unwrap();
    // No system.nim anywhere — must not escalate to an arbitrary ancestor.
    let nested = tmp.path().join("a").join("b");
    fs::create_dir_all(&nested).unwrap();
    assert!(ascend_to_lib_root(&nested).is_none());
}

#[test]
fn discover_uses_env_override_with_system_nim() {
    let tmp = tempfile::tempdir().unwrap();
    make_lib_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_NIM_SRC", tmp.path());
    let roots = discover();
    std::env::remove_var("BEARWISDOM_NIM_SRC");

    assert!(!roots.is_empty(), "override with system.nim must produce a root");
    assert_eq!(roots[0].module_path, "stdlib");
    assert_eq!(roots[0].root, tmp.path());
}

#[test]
fn discover_rejects_override_without_system_nim() {
    let tmp = tempfile::tempdir().unwrap();
    // Dir exists but has no system.nim.
    std::env::set_var("BEARWISDOM_NIM_SRC", tmp.path());
    let roots = discover();
    std::env::remove_var("BEARWISDOM_NIM_SRC");

    assert!(
        roots.iter().all(|r| r.root != tmp.path()),
        "a directory without system.nim must not qualify; real toolchain probes may still fire"
    );
}

#[test]
fn empty_dep_roots_returns_empty_index() {
    let index = build_nim_symbol_index(&[]);
    assert!(index.is_empty());
}

#[test]
fn ecosystem_identity_and_flags() {
    let e = NimStdlibEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["nim"]);
    assert!(e.uses_demand_driven_parse());
    assert!(e.supports_reachability());
    assert!(matches!(
        e.activation(),
        EcosystemActivation::LanguagePresent("nim")
    ));
}
