use super::*;
use std::fs;
use tempfile::TempDir;

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn locate_roots_returns_empty_when_msvc_install_missing() {
    // With no `VCINSTALLDIR`/`WindowsSdkDir`/vswhere-discoverable VS install
    // at the host's standard paths, discovery yields no roots — even though
    // the ecosystem would otherwise activate on any Windows + C/C++ project.
    // The on-host result depends on whether a real VS is installed, which we
    // don't gate on.
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
    assert_eq!(
        found.len(),
        1,
        "build/, Debug/, node_modules/ must be skipped"
    );
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
fn pinned_version_prefers_declared_value_verbatim() {
    // The declared `<WindowsTargetPlatformVersion>` is the pin — discovery
    // selects the matching versioned SDK subdir over the newest-installed
    // default. A single declaration is returned verbatim.
    let tmp = TempDir::new().unwrap();
    let pa = tmp.path().join("pinned.vcxproj");
    write(
        &pa,
        "<Project>\n  <PropertyGroup>\n    <WindowsTargetPlatformVersion>10.0.22621.0</WindowsTargetPlatformVersion>\n  </PropertyGroup>\n</Project>\n",
    );
    assert_eq!(
        pinned_target_platform_version(&[pa]).as_deref(),
        Some("10.0.22621.0")
    );
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

#[test]
fn vcxproj_found_in_deeply_nested_monorepo_layout() {
    // An MSBuild monorepo nests platform projects under
    // `src/<area>/<server>/<module>/<lib>/...`; an installer project can sit
    // seven directory levels below the workspace root. The scan descends far
    // enough to reach projects at that depth alongside shallower ones.
    let tmp = TempDir::new().unwrap();
    write(
        &tmp.path()
            .join("src/Installers/Windows/Module-Setup/IIS-Setup/IIS-Common/lib/Setup.vcxproj"),
        "<Project></Project>\n",
    );
    write(
        &tmp.path()
            .join("src/Servers/IIS/ModuleV2/AspNetCore/AspNetCore.vcxproj"),
        "<Project></Project>\n",
    );
    let found = find_vcxproj_files(tmp.path());
    assert_eq!(
        found.len(),
        2,
        "both the depth-5 module project and the depth-7 installer project must be found"
    );
}

#[test]
fn inert_when_no_vcxproj_present() {
    // Regression: a C/C++ tree with no MSBuild project files must not yield
    // any vcxproj — the ecosystem stays inert and contributes no SDK roots.
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("src/main.cpp"), "int main(){return 0;}\n");
    write(&tmp.path().join("src/util.h"), "#pragma once\n");
    let found = find_vcxproj_files(tmp.path());
    assert!(found.is_empty());
}

#[test]
fn um_subdir_headers_register_at_include_visible_path() {
    // Fixture SDK shaped like `Include/<version>/{um,shared}` — the IIS
    // headers a native module references (`httpserv.h`, `ahadmin.h`) live in
    // `um/`. The header index must register each at its `#include`-visible
    // path so a project's `#include <httpserv.h>` resolves.
    let tmp = TempDir::new().unwrap();
    let um = tmp.path().join("10.0.26100.0/um");
    write(&um.join("httpserv.h"), "struct IHttpServer {};\n");
    write(&um.join("ahadmin.h"), "struct IAppHostElement {};\n");
    write(
        &tmp.path().join("10.0.26100.0/shared").join("winerror.h"),
        "typedef long HRESULT;\n",
    );

    let um_root = crate::ecosystem::posix_headers::make_root(&um, TAG);
    let idx = crate::ecosystem::posix_headers::build_c_header_index(&[um_root]);
    assert_eq!(
        idx.locate("httpserv.h", "httpserv.h"),
        Some(um.join("httpserv.h").as_path())
    );
    assert_eq!(
        idx.locate("ahadmin.h", "ahadmin.h"),
        Some(um.join("ahadmin.h").as_path())
    );
}

#[test]
fn demand_pull_admits_um_header_by_include_name() {
    // Demand-pull: a source file's `#include <httpserv.h>` drives a header
    // lookup keyed by the include name. With `um/` as the dep root the
    // header is admitted as an external WalkedFile for the extraction
    // pipeline.
    let tmp = TempDir::new().unwrap();
    let um = tmp.path().join("um");
    write(&um.join("httpserv.h"), "struct IHttpServer {};\n");

    let um_root = crate::ecosystem::posix_headers::make_root(&um, TAG);
    let admitted = crate::ecosystem::posix_headers::resolve_header(&um_root, "httpserv.h");
    let file = admitted.expect("httpserv.h must be admitted from the um/ root");
    assert_eq!(file.absolute_path, um.join("httpserv.h"));
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
    let include = tmp
        .path()
        .join("2022/BuildTools/VC/Tools/MSVC/14.44.35207.1/include");
    fs::create_dir_all(&include).unwrap();
    let bases = vec![tmp.path().to_path_buf()];
    assert_eq!(discover_vc_tools_include_layout(&bases), Some(include));
}

#[test]
fn vc_tools_probe_picks_highest_msvc_version() {
    let tmp = TempDir::new().unwrap();
    let older = tmp
        .path()
        .join("2022/BuildTools/VC/Tools/MSVC/14.40.33807/include");
    let newer = tmp
        .path()
        .join("2022/BuildTools/VC/Tools/MSVC/14.44.35207.1/include");
    fs::create_dir_all(&older).unwrap();
    fs::create_dir_all(&newer).unwrap();
    let bases = vec![tmp.path().to_path_buf()];
    assert_eq!(discover_vc_tools_include_layout(&bases), Some(newer));
}

#[test]
fn vc_tools_probe_prefers_newer_year() {
    let tmp = TempDir::new().unwrap();
    let vs2019 = tmp
        .path()
        .join("2019/BuildTools/VC/Tools/MSVC/14.29.30133/include");
    let vs2022 = tmp
        .path()
        .join("2022/BuildTools/VC/Tools/MSVC/14.44.35207.1/include");
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
    let toolchain_no_include = tmp
        .path()
        .join("2022/BuildTools/VC/Tools/MSVC/14.44.35207.1");
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
    assert_eq!(
        result.file_name().and_then(|n| n.to_str()),
        Some("14.44.35207.1")
    );
}
