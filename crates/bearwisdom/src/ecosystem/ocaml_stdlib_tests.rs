use super::*;
use std::fs;

/// Lay down a minimal OCaml stdlib fixture: `list.mli` is the marker, plus a
/// handful of other interface/impl files.
fn make_stdlib_fixture(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("list.mli"), "val map : ('a -> 'b) -> 'a list -> 'b list\n").unwrap();
    fs::write(dir.join("list.ml"), "let map f l = ()\n").unwrap();
    fs::write(dir.join("string.mli"), "val length : string -> int\n").unwrap();
    fs::write(dir.join("stringLabels.mli"), "val get : string -> int -> char\n").unwrap();
    fs::write(dir.join("array.ml"), "let length a = ()\n").unwrap();
    // Noise: C runtime headers under caml/ must be skipped.
    let caml = dir.join("caml");
    fs::create_dir_all(&caml).unwrap();
    fs::write(caml.join("mlvalues.ml"), "(* not stdlib *)\n").unwrap();
}

#[test]
fn walk_yields_ml_and_mli_files() {
    let tmp = tempfile::tempdir().unwrap();
    make_stdlib_fixture(tmp.path());

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
    assert!(names.contains(&"list.mli".to_string()), "{names:?}");
    assert!(names.contains(&"string.mli".to_string()), "{names:?}");
    assert!(names.contains(&"array.ml".to_string()), "{names:?}");
    // caml/ contents must be pruned.
    assert!(
        !names.contains(&"mlvalues.ml".to_string()),
        "caml/ runtime headers must be skipped: {names:?}"
    );
    for f in &files {
        assert_eq!(f.language, "ocaml");
        assert!(f.relative_path.starts_with("ext:ocaml:"));
    }
}

#[test]
fn module_name_capitalizes_base_filename() {
    assert_eq!(
        module_name_from_path(Path::new("/x/list.mli")).as_deref(),
        Some("List")
    );
    assert_eq!(
        module_name_from_path(Path::new("/x/stringLabels.mli")).as_deref(),
        Some("StringLabels")
    );
    assert_eq!(
        module_name_from_path(Path::new("/x/array.ml")).as_deref(),
        Some("Array")
    );
}

#[test]
fn symbol_index_maps_module_names_interface_preferred() {
    let tmp = tempfile::tempdir().unwrap();
    make_stdlib_fixture(tmp.path());

    let dep = ExternalDepRoot {
        module_path: "stdlib".into(),
        version: String::new(),
        root: tmp.path().to_path_buf(),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let index = build_ocaml_symbol_index(&[dep]);

    // List is reachable both under the stdlib module and by its own name.
    let list = index.locate("stdlib", "List");
    assert!(list.is_some(), "List not indexed; {} entries", index.len());
    // .mli interface wins over .ml when both exist.
    assert!(
        list.unwrap().to_string_lossy().ends_with("list.mli"),
        "interface must win over implementation"
    );
    assert!(index.locate("stdlib", "String").is_some());
    assert!(index.locate("stdlib", "StringLabels").is_some());
    assert!(index.locate("stdlib", "Array").is_some());
}

#[test]
fn discover_uses_env_override_with_list_mli() {
    let tmp = tempfile::tempdir().unwrap();
    make_stdlib_fixture(tmp.path());

    std::env::set_var("BEARWISDOM_OCAML_SRC", tmp.path());
    let roots = discover();
    std::env::remove_var("BEARWISDOM_OCAML_SRC");

    assert!(!roots.is_empty(), "override with list.mli must produce a root");
    assert_eq!(roots[0].module_path, "stdlib");
    assert_eq!(roots[0].root, tmp.path());
}

#[test]
fn discover_rejects_override_without_list_marker() {
    let tmp = tempfile::tempdir().unwrap();
    // Dir exists but has no list.mli/list.ml.
    std::env::set_var("BEARWISDOM_OCAML_SRC", tmp.path());
    let roots = discover();
    std::env::remove_var("BEARWISDOM_OCAML_SRC");

    assert!(
        roots.iter().all(|r| r.root != tmp.path()),
        "a directory without list.mli must not qualify; real toolchain probes may still fire"
    );
}

#[test]
fn empty_dep_roots_returns_empty_index() {
    let index = build_ocaml_symbol_index(&[]);
    assert!(index.is_empty());
}

#[test]
fn ecosystem_identity_and_flags() {
    let e = OcamlStdlibEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["ocaml"]);
    assert!(e.uses_demand_driven_parse());
    assert!(e.supports_reachability());
    assert!(matches!(
        e.activation(),
        EcosystemActivation::LanguagePresent("ocaml")
    ));
}
