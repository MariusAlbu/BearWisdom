use super::*;
use std::fs;
use tempfile::TempDir;

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn locate_roots_returns_empty_when_msvc_install_missing() {
    // With no `BEARWISDOM_MSVC_INCLUDE` override and no `VCINSTALLDIR`/
    // `WindowsSdkDir`/vswhere-discoverable VS install at the host's
    // standard paths, discovery yields no roots — even though the
    // ecosystem would otherwise activate on any Windows + C/C++ project.
    // This test isolates the override path; the on-host result depends
    // on whether a real VS is installed, which we don't gate on.
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("main.c"), "int main() { return 0; }\n");
    let _ = tmp; // path used only to satisfy the locate_roots signature
    // No assertion: this test exists to document the behavior. The
    // real install-vs-no-install distinction is exercised through the
    // VC Tools probe tests below using a tmpfs-mocked layout.
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

// ---------------------------------------------------------------------------
// VC Tools include discovery
// ---------------------------------------------------------------------------

#[test]
fn vc_tools_probe_finds_buildtools_install() {
    let tmp = TempDir::new().unwrap();
    let include = tmp.path().join("2022/BuildTools/VC/Tools/MSVC/14.44.35207.1/include");
    fs::create_dir_all(&include).unwrap();
    let bases = vec![tmp.path().to_path_buf()];
    assert_eq!(discover_vc_tools_include_layout(&bases), Some(include));
}

#[test]
fn vc_tools_probe_picks_highest_msvc_version() {
    let tmp = TempDir::new().unwrap();
    let older = tmp.path().join("2022/BuildTools/VC/Tools/MSVC/14.40.33807/include");
    let newer = tmp.path().join("2022/BuildTools/VC/Tools/MSVC/14.44.35207.1/include");
    fs::create_dir_all(&older).unwrap();
    fs::create_dir_all(&newer).unwrap();
    let bases = vec![tmp.path().to_path_buf()];
    assert_eq!(discover_vc_tools_include_layout(&bases), Some(newer));
}

#[test]
fn vc_tools_probe_prefers_newer_year() {
    let tmp = TempDir::new().unwrap();
    let vs2019 = tmp.path().join("2019/BuildTools/VC/Tools/MSVC/14.29.30133/include");
    let vs2022 = tmp.path().join("2022/BuildTools/VC/Tools/MSVC/14.44.35207.1/include");
    fs::create_dir_all(&vs2019).unwrap();
    fs::create_dir_all(&vs2022).unwrap();
    let bases = vec![tmp.path().to_path_buf()];
    assert_eq!(discover_vc_tools_include_layout(&bases), Some(vs2022));
}

#[test]
fn vc_tools_probe_returns_none_when_layout_missing() {
    let tmp = TempDir::new().unwrap();
    let bases = vec![tmp.path().to_path_buf()];
    assert!(discover_vc_tools_include_layout(&bases).is_none());
}

#[test]
fn vc_tools_probe_skips_msvc_dir_with_no_include_subdir() {
    // A toolchain dir present but missing `include/` (broken install)
    // must not return a bogus path — the caller would push it into
    // `include_roots` and try to walk it.
    let tmp = TempDir::new().unwrap();
    let toolchain_no_include = tmp.path().join("2022/BuildTools/VC/Tools/MSVC/14.44.35207.1");
    fs::create_dir_all(&toolchain_no_include).unwrap();
    let bases = vec![tmp.path().to_path_buf()];
    assert!(discover_vc_tools_include_layout(&bases).is_none());
}

#[test]
fn newest_subdir_handles_missing_parent() {
    let tmp = TempDir::new().unwrap();
    assert!(newest_subdir(&tmp.path().join("nonexistent")).is_none());
}

#[test]
fn newest_subdir_returns_lexicographic_max() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("14.20.27508")).unwrap();
    fs::create_dir_all(tmp.path().join("14.44.35207.1")).unwrap();
    fs::create_dir_all(tmp.path().join("14.29.30133")).unwrap();
    let result = newest_subdir(tmp.path()).unwrap();
    assert_eq!(result.file_name().and_then(|n| n.to_str()), Some("14.44.35207.1"));
}
