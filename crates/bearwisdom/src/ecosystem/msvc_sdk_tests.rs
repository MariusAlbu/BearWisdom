use super::*;
use std::fs;
use tempfile::TempDir;

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn no_vcxproj_returns_empty_dep_roots() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("main.c"), "int main() { return 0; }\n");
    let eco = MsvcSdkEcosystem;
    let roots = ExternalSourceLocator::locate_roots(&eco, tmp.path());
    assert!(
        roots.is_empty(),
        "without a vcxproj the msvc-sdk ecosystem must not probe the SDK"
    );
}

#[test]
fn vcxproj_extension_match_is_case_insensitive() {
    let tmp = TempDir::new().unwrap();
    write(
        &tmp.path().join("App.VcxProj"),
        "<Project xmlns=\"http://schemas.microsoft.com/developer/msbuild/2003\"></Project>\n",
    );
    let found = find_vcxproj_files(tmp.path());
    assert_eq!(found.len(), 1);
}

#[test]
fn vcxproj_walk_skips_well_known_build_dirs() {
    let tmp = TempDir::new().unwrap();
    let body = "<Project></Project>\n";
    write(&tmp.path().join("real/MyApp.vcxproj"), body);
    write(&tmp.path().join("build/Cached.vcxproj"), body);
    write(&tmp.path().join("node_modules/unused.vcxproj"), body);
    write(&tmp.path().join("Debug/Cached.vcxproj"), body);
    let found = find_vcxproj_files(tmp.path());
    assert_eq!(found.len(), 1, "build/, Debug/, node_modules/ must be skipped");
    assert!(found[0].to_string_lossy().contains("real"));
}

#[test]
fn pinned_version_extracts_highest_target_platform_version() {
    let tmp = TempDir::new().unwrap();
    let pa = tmp.path().join("a.vcxproj");
    let pb = tmp.path().join("b.vcxproj");
    write(
        &pa,
        "<Project>\n  <PropertyGroup>\n    <WindowsTargetPlatformVersion>10.0.22621.0</WindowsTargetPlatformVersion>\n  </PropertyGroup>\n</Project>\n",
    );
    write(
        &pb,
        "<Project>\n  <PropertyGroup>\n    <WindowsTargetPlatformVersion>10.0.26100.0</WindowsTargetPlatformVersion>\n  </PropertyGroup>\n</Project>\n",
    );
    let pinned = pinned_target_platform_version(&[pa, pb]);
    assert_eq!(pinned.as_deref(), Some("10.0.26100.0"));
}

#[test]
fn pinned_version_returns_none_when_no_vcxproj_declares_one() {
    let tmp = TempDir::new().unwrap();
    let pa = tmp.path().join("legacy.vcxproj");
    write(
        &pa,
        "<Project>\n  <PropertyGroup>\n    <ConfigurationType>Application</ConfigurationType>\n  </PropertyGroup>\n</Project>\n",
    );
    let pinned = pinned_target_platform_version(&[pa]);
    assert!(pinned.is_none());
}

#[test]
fn pinned_version_ignores_blank_value() {
    let tmp = TempDir::new().unwrap();
    let pa = tmp.path().join("blank.vcxproj");
    write(
        &pa,
        "<Project><WindowsTargetPlatformVersion></WindowsTargetPlatformVersion></Project>\n",
    );
    assert!(pinned_target_platform_version(&[pa]).is_none());
}

#[test]
fn vcxproj_present_at_nested_depth() {
    // Discovery walks past directory boundaries up to the configured cap.
    let tmp = TempDir::new().unwrap();
    write(
        &tmp.path().join("apps/native/win/MyApp.vcxproj"),
        "<Project></Project>\n",
    );
    let found = find_vcxproj_files(tmp.path());
    assert_eq!(found.len(), 1);
}

#[cfg(not(target_os = "windows"))]
#[test]
fn discover_returns_empty_off_windows() {
    // The dep-root probe is short-circuited on non-Windows hosts so the
    // ecosystem stays inert in CI on Linux/macOS even when the env vars
    // happen to be set.
    let roots = discover_msvc_include(Some("10.0.26100.0"));
    assert!(roots.is_empty());
}

#[test]
fn ecosystem_declares_demand_driven() {
    assert!(MsvcSdkEcosystem.uses_demand_driven_parse());
    assert!(MsvcSdkEcosystem.supports_reachability());
}

#[test]
fn walk_root_is_empty_under_demand_driven() {
    let tmp = TempDir::new().unwrap();
    let dep = crate::ecosystem::posix_headers::make_root(tmp.path(), TAG);
    assert!(Ecosystem::walk_root(&MsvcSdkEcosystem, &dep).is_empty());
}
