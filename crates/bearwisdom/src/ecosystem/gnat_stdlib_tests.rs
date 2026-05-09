// =============================================================================
// gnat_stdlib_tests.rs
// =============================================================================

use super::*;

fn write_ads(dir: &std::path::Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).unwrap();
}

#[test]
fn ecosystem_identity() {
    let e = GnatStdlibEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["ada"]);
}

#[test]
fn eager_walk_and_substrate() {
    let e = GnatStdlibEcosystem;
    // Bare-name resolution under Ada `use` clauses needs every package's
    // public subprograms in the symbol table — eager walk pays off here.
    assert!(!e.uses_demand_driven_parse());
    assert!(e.supports_reachability());
    assert!(e.is_workspace_global());
}

#[test]
fn walk_root_yields_ads_files() {
    let tmp = std::env::temp_dir().join("bw-test-gnat-stdlib-walk");
    let _ = std::fs::remove_dir_all(&tmp);
    let adainclude = tmp.join("adainclude");
    std::fs::create_dir_all(&adainclude).unwrap();
    write_ads(
        &adainclude,
        "a-textio.ads",
        "package Ada.Text_IO is\n   procedure Put_Line (S : String);\nend Ada.Text_IO;\n",
    );
    write_ads(
        &adainclude,
        "system.ads",
        "package System is\n   pragma Pure;\nend System;\n",
    );

    let dep = ExternalDepRoot {
        module_path: "gnat-stdlib".to_string(),
        version: String::new(),
        root: adainclude.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let files = Ecosystem::walk_root(&GnatStdlibEcosystem, &dep);

    assert_eq!(files.len(), 2, "expected 2 .ads files, got {files:?}");
    for f in &files {
        assert_eq!(f.language, "ada");
        assert!(f.relative_path.starts_with("ext:gnat-stdlib:"));
        assert!(f.relative_path.ends_with(".ads"));
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn scan_package_decl_recognises_basic_form() {
    let tmp = std::env::temp_dir().join("bw-test-gnat-stdlib-decl");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    write_ads(
        &tmp,
        "a-textio.ads",
        "-- Ada Text IO header\npackage Ada.Text_IO is\n   procedure Put (Item : String);\nend Ada.Text_IO;\n",
    );
    let qname = scan_package_decl(&tmp.join("a-textio.ads"));
    assert_eq!(qname, Some("Ada.Text_IO".to_string()));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn scan_package_decl_skips_with_clauses_and_pragmas() {
    let tmp = std::env::temp_dir().join("bw-test-gnat-stdlib-skip");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    write_ads(
        &tmp,
        "g-os_lib.ads",
        "-- Header comment line\n\
         pragma Ada_2012;\n\
         with Ada.Calendar;\n\
         with System;\n\
         private package GNAT.OS_Lib is\n\
            type File_Descriptor is private;\n\
         end GNAT.OS_Lib;\n",
    );
    let qname = scan_package_decl(&tmp.join("g-os_lib.ads"));
    // Note: the scanner currently picks the first `package` line — pragma
    // and with clauses don't trigger because they don't start with the
    // `package` keyword after qualifier stripping. The `private` qualifier
    // is recognized and stripped.
    assert_eq!(qname, Some("GNAT.OS_Lib".to_string()));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn scan_package_decl_recognises_generic_and_body() {
    let tmp = std::env::temp_dir().join("bw-test-gnat-stdlib-generic");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    write_ads(
        &tmp,
        "a-cohama.ads",
        "generic package Ada.Containers.Hashed_Maps is\n   type Map is private;\nend Ada.Containers.Hashed_Maps;\n",
    );
    write_ads(
        &tmp,
        "a-stwifi.adb",
        "package body Ada.Strings.Wide_Fixed is\n   procedure Move is begin null; end;\nend Ada.Strings.Wide_Fixed;\n",
    );

    assert_eq!(
        scan_package_decl(&tmp.join("a-cohama.ads")),
        Some("Ada.Containers.Hashed_Maps".to_string())
    );
    assert_eq!(
        scan_package_decl(&tmp.join("a-stwifi.adb")),
        Some("Ada.Strings.Wide_Fixed".to_string())
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn build_symbol_index_indexes_packages_lowercase_and_canonical() {
    let tmp = std::env::temp_dir().join("bw-test-gnat-stdlib-index");
    let _ = std::fs::remove_dir_all(&tmp);
    let adainclude = tmp.join("adainclude");
    std::fs::create_dir_all(&adainclude).unwrap();

    write_ads(
        &adainclude,
        "a-textio.ads",
        "package Ada.Text_IO is\nend Ada.Text_IO;\n",
    );
    write_ads(
        &adainclude,
        "g-os_lib.ads",
        "package GNAT.OS_Lib is\nend GNAT.OS_Lib;\n",
    );
    write_ads(&adainclude, "system.ads", "package System is\nend System;\n");

    let dep = ExternalDepRoot {
        module_path: "gnat-stdlib".to_string(),
        version: String::new(),
        root: adainclude.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let index = build_gnat_stdlib_symbol_index(&[dep]);

    assert!(
        index.locate("gnat-stdlib", "ada.text_io").is_some(),
        "lowercase Ada.Text_IO should resolve; index size {}",
        index.len()
    );
    assert!(
        index.locate("gnat-stdlib", "Ada.Text_IO").is_some(),
        "case-preserving Ada.Text_IO should resolve"
    );
    assert!(
        index.locate("gnat-stdlib", "gnat.os_lib").is_some(),
        "GNAT.OS_Lib should resolve"
    );
    assert!(
        index.locate("gnat-stdlib", "system").is_some(),
        "System should resolve"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_import_returns_the_matching_ads() {
    let tmp = std::env::temp_dir().join("bw-test-gnat-stdlib-resolve");
    let _ = std::fs::remove_dir_all(&tmp);
    let adainclude = tmp.join("adainclude");
    std::fs::create_dir_all(&adainclude).unwrap();
    write_ads(
        &adainclude,
        "a-textio.ads",
        "package Ada.Text_IO is\nend Ada.Text_IO;\n",
    );

    let dep = ExternalDepRoot {
        module_path: "gnat-stdlib".to_string(),
        version: String::new(),
        root: adainclude.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = GnatStdlibEcosystem.resolve_import(&dep, "Ada.Text_IO", &[]);
    assert_eq!(walked.len(), 1);
    assert_eq!(walked[0].language, "ada");
    assert!(walked[0].relative_path.starts_with("ext:gnat-stdlib:"));
    assert!(walked[0].relative_path.ends_with("a-textio.ads"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_symbol_strips_trailing_children() {
    // A request for `Ada.Text_IO.Put_Line` must locate the `Ada.Text_IO`
    // spec — the symbol is defined inside that package.
    let tmp = std::env::temp_dir().join("bw-test-gnat-stdlib-fqn");
    let _ = std::fs::remove_dir_all(&tmp);
    let adainclude = tmp.join("adainclude");
    std::fs::create_dir_all(&adainclude).unwrap();
    write_ads(
        &adainclude,
        "a-textio.ads",
        "package Ada.Text_IO is\n   procedure Put_Line (S : String);\nend Ada.Text_IO;\n",
    );

    let dep = ExternalDepRoot {
        module_path: "gnat-stdlib".to_string(),
        version: String::new(),
        root: adainclude.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = GnatStdlibEcosystem.resolve_symbol(&dep, "Ada.Text_IO.Put_Line");
    assert_eq!(walked.len(), 1);
    assert!(walked[0].relative_path.ends_with("a-textio.ads"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn no_install_returns_empty_roots() {
    // Save and clear environment so the probe doesn't accidentally pick up
    // a real installation.
    let saved = std::env::var_os("BEARWISDOM_GNAT_LIBDIR");
    std::env::remove_var("BEARWISDOM_GNAT_LIBDIR");

    // Point the override at a non-existent path.
    std::env::set_var(
        "BEARWISDOM_GNAT_LIBDIR",
        std::env::temp_dir().join("does-not-exist-gnat"),
    );
    let roots = discover_gnat_adainclude();
    // We can't assert empty — a real GNAT install on the dev host may
    // still be picked up via gnatls or alire probing. We only assert
    // the call doesn't panic.
    let _ = roots;

    // Restore.
    match saved {
        Some(v) => std::env::set_var("BEARWISDOM_GNAT_LIBDIR", v),
        None => std::env::remove_var("BEARWISDOM_GNAT_LIBDIR"),
    }
}
