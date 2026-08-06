// =============================================================================
// nuget/dll_locator_tests.rs — unit tests for NuGet coordinate discovery
// =============================================================================

use super::*;

/// Write `content` as `<dir>/obj/project.assets.json` under a fresh temp dir.
fn temp_project_with_assets(content: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let obj = dir.path().join("obj");
    std::fs::create_dir_all(&obj).expect("mkdir obj");
    std::fs::write(obj.join("project.assets.json"), content).expect("write assets");
    dir
}

#[test]
fn assets_json_yields_transitive_package_coords() {
    // The restore artifact lists the FULL transitive closure — packages the
    // csproj never names directly (the ones declaring IdentityUser et al).
    let dir = temp_project_with_assets(
        r#"{
            "libraries": {
                "FluentValidation/12.1.1": {"type": "package"},
                "Microsoft.Extensions.Identity.Stores/9.0.0": {"type": "package"},
                "Equinox.Domain/1.0.0": {"type": "project"}
            }
        }"#,
    );
    let mut coords = collect_transitive_coords_from_assets_json(dir.path());
    coords.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(coords.len(), 2, "project-type libraries are not packages");
    assert_eq!(coords[0].name, "FluentValidation");
    assert_eq!(coords[0].version.as_deref(), Some("12.1.1"));
    assert_eq!(coords[1].name, "Microsoft.Extensions.Identity.Stores");
}

#[test]
fn missing_or_malformed_assets_json_yields_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(collect_transitive_coords_from_assets_json(dir.path()).is_empty());
    let bad = temp_project_with_assets("{ not json");
    assert!(collect_transitive_coords_from_assets_json(bad.path()).is_empty());
}



#[test]
fn find_dlls_returns_all_tfm_assets_primary_first() {
    // A package's assembly names are independent of its package id: the
    // package `pkg.mtp-v1` here ships `pkg.core.dll` + `pkg.mtp-v1.dll`.
    // Every lib/<tfm> DLL is a compile asset; the id-matching one sorts first.
    let dir = tempfile::tempdir().expect("tempdir");
    let tfm = dir.path().join("lib").join("net8.0");
    std::fs::create_dir_all(&tfm).expect("mkdir tfm");
    std::fs::write(tfm.join("pkg.core.dll"), b"x").unwrap();
    std::fs::write(tfm.join("Pkg.Mtp-V1.dll"), b"x").unwrap();
    std::fs::write(tfm.join("notes.txt"), b"x").unwrap();
    let dlls = find_dlls_in_version_dir(dir.path(), "pkg.mtp-v1");
    let names: Vec<String> = dlls
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["Pkg.Mtp-V1.dll", "pkg.core.dll"]);
}

