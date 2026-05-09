// =============================================================================
// ecosystem/msvc_sdk.rs — MSVC C/C++ SDK headers (Windows stdlib ecosystem)
//
// Provides Windows SDK + Visual C++ Runtime headers as an external dep root
// for projects that build with MSBuild. Activates only on Windows hosts and
// only when a *.vcxproj file is present in the project — non-MSBuild C/C++
// projects (Makefile, plain CMake without compile_commands) do not pull
// MSVC headers from this ecosystem.
//
// Demand-driven parsing
// ---------------------
// `walk_root` returns empty; `build_symbol_index` enumerates header files
// under each dep root and registers each at its `#include`-visible path.
// The Stage-2 demand loop pulls only the headers a project's source
// actually `#include`s. The Windows SDK Include/ dir has five top-level
// children with wildly different footprints (ucrt: 66 headers, um: 1.5k,
// shared: 280, winrt: 400, cppwinrt: 1.4k — ~3.7k total); the demand-driven
// model keeps the parsed slice scoped to the include graph the project
// actually reaches.
//
// Project gate
// ------------
// `locate_roots` scans the project for `*.vcxproj` files. When none are
// found and `compile_commands.json` is also absent, the project is not
// using MSBuild — return empty. When one or more vcxproj files are found,
// the highest declared `<WindowsTargetPlatformVersion>` value pins the
// SDK version sub-directory under `WindowsSdkDir/Include/`. Falls back to
// the newest installed SDK if no version pin can be parsed.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
    Platform, SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("msvc-sdk");
const TAG: &str = "msvc-sdk";

pub struct MsvcSdkEcosystem;

impl Ecosystem for MsvcSdkEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { &["c", "cpp"] }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::All(&[
            EcosystemActivation::AlwaysOnPlatform(Platform::Windows),
            EcosystemActivation::Any(&[
                EcosystemActivation::LanguagePresent("c"),
                EcosystemActivation::LanguagePresent("cpp"),
            ]),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        // The activation rule (Windows + C/C++) already gates this; on a
        // Windows host MSVC SDK is the universal C runtime for every
        // C/C++ project (Unix-style code reaches `<stdio.h>` /
        // `<immintrin.h>` / `<intrin.h>` through MSVC ucrt + intrinsic
        // headers). Pin the SDK version from `*.vcxproj`'s
        // `<WindowsTargetPlatformVersion>` when present; otherwise pick
        // the newest installed SDK.
        let vcxprojs = find_vcxproj_files(ctx.project_root);
        let pinned_version = pinned_target_platform_version(&vcxprojs);
        discover_msvc_include(pinned_version.as_deref())
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn supports_reachability(&self) -> bool { true }
    fn uses_demand_driven_parse(&self) -> bool { true }

    // Windows SDK is workspace-level: the host's installed Windows SDK
    // serves every C/C++ translation unit in the build.
    fn is_workspace_global(&self) -> bool { true }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        super::posix_headers::build_c_header_index(dep_roots)
    }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        header: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        super::posix_headers::resolve_header(dep, header).into_iter().collect()
    }

    fn resolve_symbol(&self, dep: &ExternalDepRoot, fqn: &str) -> Vec<WalkedFile> {
        super::posix_headers::resolve_header(dep, fqn).into_iter().collect()
    }
}

impl ExternalSourceLocator for MsvcSdkEcosystem {
    fn ecosystem(&self) -> &'static str { TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        let vcxprojs = find_vcxproj_files(project_root);
        let pinned_version = pinned_target_platform_version(&vcxprojs);
        discover_msvc_include(pinned_version.as_deref())
    }
    fn locate_roots_for_package(
        &self,
        workspace_root: &Path,
        _package_abs_path: &Path,
        _package_id: i64,
    ) -> Vec<ExternalDepRoot> {
        let vcxprojs = find_vcxproj_files(workspace_root);
        let pinned_version = pinned_target_platform_version(&vcxprojs);
        discover_msvc_include(pinned_version.as_deref())
    }
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<MsvcSdkEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(MsvcSdkEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// vcxproj scanning + version pin
// ---------------------------------------------------------------------------
//
// A `*.vcxproj` file declares the SDK version pin via
// `<WindowsTargetPlatformVersion>`. When present, the highest declared
// version is used to pick a sub-directory under `WindowsSdkDir/Include/`.
// Absence of vcxproj does not skip MSVC SDK probing — every C/C++ project
// on a Windows host resolves stdlib through MSVC ucrt. C stdlib functions
// (`memset`, `printf`), Intel intrinsics (`__m256i`), and Win32 SDK types
// (`HWND`,
//     `WCHAR`) stay unresolved on every CMake-on-MSVC project.
//
// Note: this overlaps with `compile-commands` for the same project,
// which is fine — both ecosystems contribute distinct dep roots and the
// symbol-index merge handles dedup. The non-MSVC `compile-commands`
// path does not suppress msvc-sdk, because compile_commands.json
// doesn't list SDK paths on MSVC.
//

/// Find every `*.vcxproj` under `project_root`, capped at depth 6 to avoid
/// pathological monorepos. Returns absolute paths.
fn find_vcxproj_files(project_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_vcxproj_rec(project_root, &mut out, 0);
    out
}

fn walk_vcxproj_rec(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    if depth >= 6 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            // Skip well-known build / dependency / VCS directories so the
            // walk stays cheap on large repos.
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name,
                    ".git" | ".hg" | ".svn" | "node_modules" | "target" |
                    "build" | "out" | "bin" | "obj" | "Debug" | "Release" |
                    ".vs" | "packages" | "vendor"
                ) {
                    continue;
                }
            }
            walk_vcxproj_rec(&path, out, depth + 1);
            continue;
        }
        if !ft.is_file() { continue }
        if path.extension().and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("vcxproj"))
        {
            out.push(path);
        }
    }
}

/// Read every vcxproj's `<WindowsTargetPlatformVersion>` element and return
/// the highest version string declared. Returns `None` when no vcxproj
/// declares one (rare; older Visual Studio formats omit it).
fn pinned_target_platform_version(vcxprojs: &[PathBuf]) -> Option<String> {
    let mut versions: Vec<String> = Vec::new();
    for path in vcxprojs {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        for line in text.lines() {
            let trim = line.trim();
            let open = "<WindowsTargetPlatformVersion>";
            let close = "</WindowsTargetPlatformVersion>";
            if let Some(start) = trim.find(open) {
                let after = &trim[start + open.len()..];
                if let Some(end) = after.find(close) {
                    let v = after[..end].trim();
                    if !v.is_empty() {
                        versions.push(v.to_string());
                    }
                }
            }
        }
    }
    versions.sort();
    versions.into_iter().next_back()
}

// ---------------------------------------------------------------------------
// Windows SDK discovery
// ---------------------------------------------------------------------------

/// Enumerate candidate MSVC Include roots. Each sub-dir of
/// `WindowsSdkDir/Include/<version>/` (ucrt, um, shared, winrt, cppwinrt)
/// becomes its own `ExternalDepRoot` so the `#include`-visible path is the
/// header's path relative to that sub-dir — matching what a user's
/// `#include <stdio.h>` statement names. The `cppwinrt/` and `winrt/` dirs
/// are included for completeness; demand-driven parsing won't pay for them
/// unless the project's source actually references them.
///
/// `pinned_version` is the value declared in the project's vcxproj
/// `<WindowsTargetPlatformVersion>`. When provided and a matching
/// versioned subdir exists under the SDK include root, that subdir is
/// preferred over the newest-installed default.
fn discover_msvc_include(pinned_version: Option<&str>) -> Vec<ExternalDepRoot> {
    if !cfg!(target_os = "windows") { return Vec::new() }

    if let Some(explicit) = std::env::var_os("BEARWISDOM_MSVC_INCLUDE") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            return vec![super::posix_headers::make_root(&p, TAG)];
        }
    }

    let mut include_roots: Vec<PathBuf> = Vec::new();
    if let Some(vc) = std::env::var_os("VCINSTALLDIR") {
        let p = PathBuf::from(vc).join("include");
        if p.is_dir() {
            include_roots.push(p);
        }
    }
    // Fallback when VCINSTALLDIR isn't set in the parent shell — the
    // typical case when `bw` runs outside a vcvarsall-sourced
    // environment (the configure-msvc.bat scripts source vcvars in a
    // child cmd, so VCINSTALLDIR doesn't propagate to bw). Probe the
    // conventional Visual Studio install layout for VC Tools headers
    // so C++ stdlib types (`<vector>`, `<memory>`, `<string>`) resolve
    // on CMake-on-MSVC projects. Only reached when the project gate
    // already established this is an MSVC-flavoured build, so the
    // probe stays bounded to projects that need it.
    if include_roots.is_empty() {
        if let Some(p) = discover_vc_tools_include(&default_vs_install_bases()) {
            include_roots.push(p);
        }
    }
    let wdk_include_root: Option<PathBuf> = std::env::var_os("WindowsSdkDir")
        .map(|wdk| PathBuf::from(wdk).join("Include"))
        .filter(|p| p.is_dir())
        .or_else(|| {
            // Conventional Windows 10/11 SDK install path. Read
            // `ProgramFiles(x86)` from the environment so the path
            // works on any Windows host (drive letter, locale, custom
            // Program Files location).
            let pf86 = std::env::var_os("ProgramFiles(x86)")?;
            let p = PathBuf::from(pf86)
                .join("Windows Kits")
                .join("10")
                .join("Include");
            if p.is_dir() { Some(p) } else { None }
        });
    if let Some(include_root) = wdk_include_root {
        if let Some(pinned) = pinned_version {
            let pinned_dir = include_root.join(pinned);
            if pinned_dir.is_dir() {
                include_roots.push(pinned_dir);
            } else {
                debug!("msvc-sdk: pinned version {pinned} not installed; falling back to newest");
                include_roots.extend(super::posix_headers::newest_sdk_versions(&include_root));
            }
        } else {
            include_roots.extend(super::posix_headers::newest_sdk_versions(&include_root));
        }
    }

    if include_roots.is_empty() {
        debug!("msvc-sdk: no VCINSTALLDIR / WindowsSdkDir / override probed");
        return Vec::new();
    }

    let mut out = Vec::new();
    for include in &include_roots {
        let structured_children: Vec<&str> = ["ucrt", "um", "shared", "winrt", "cppwinrt"]
            .iter()
            .copied()
            .filter(|sub| include.join(sub).is_dir())
            .collect();
        if structured_children.is_empty() {
            // VCINSTALLDIR/include is flat (C++ stdlib: cstdio, iostream, ...).
            out.push(super::posix_headers::make_root(include, TAG));
            continue;
        }
        for sub in structured_children {
            out.push(super::posix_headers::make_root(&include.join(sub), TAG));
        }
        // The version root itself contains `winrt/Foo.h` under its root, and
        // compilers walk through when resolving `#include <winrt/Foo.h>`.
        out.push(super::posix_headers::make_root(include, TAG));
    }
    out
}

// ---------------------------------------------------------------------------
// VC Tools include discovery
// ---------------------------------------------------------------------------
//
// Visual Studio installs lay out the C++ toolchain headers at:
//   <base>/<year>/<sku>/VC/Tools/MSVC/<version>/include/
// where:
//   <base>  = "C:/Program Files (x86)/Microsoft Visual Studio"
//             (or the 64-bit "C:/Program Files/..." variant)
//   <year>  = "2022", "2019", "2017"
//   <sku>   = "BuildTools" | "Enterprise" | "Professional" | "Community"
//   <ver>   = "14.44.35207.1" — the toolchain version, dotted
//
// `vcvarsall.bat amd64` resolves and exports this as `VCINSTALLDIR`.
// When the user runs `bw` outside that environment (most
// `configure-msvc.bat` scripts only source vcvars inside a child
// cmd), VCINSTALLDIR is empty and the C++ stdlib disappears from the
// resolution graph. Probing the conventional layout recovers it.

fn default_vs_install_bases() -> Vec<PathBuf> {
    vec![
        PathBuf::from("C:/Program Files (x86)/Microsoft Visual Studio"),
        PathBuf::from("C:/Program Files/Microsoft Visual Studio"),
    ]
}

/// Locate the newest VC Tools include directory.
///
/// Discovery order:
///   1. **`vswhere.exe`** — Microsoft's official VS install discovery
///      tool (ships with every VS Installer 2017+ at a fixed path).
///      Returns the install path of the latest VS instance with the
///      VC compilers component, regardless of edition (BuildTools /
///      Community / Professional / Enterprise) or release year.
///   2. **Conventional `Microsoft Visual Studio` directory layout** —
///      fallback for unusual installs (older VS, custom relocation,
///      vswhere absent). Walks `<base>/<year>/<edition>/VC/Tools/MSVC`
///      generically — the year and edition directories are read from
///      disk, not hardcoded, so any future VS release is picked up.
///
/// Within a `VC/Tools/MSVC` directory the highest version subdirectory
/// wins. Versions follow `14.X.YYYYY` since VS2017; `numeric_sort`
/// handles the two-digit X bumps correctly.
fn discover_vc_tools_include(bases: &[PathBuf]) -> Option<PathBuf> {
    if let Some(p) = vswhere_vc_tools_include() {
        return Some(p);
    }
    discover_vc_tools_include_layout(bases)
}

/// Run `vswhere.exe -latest -products * -requires
/// Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`
/// to find the newest VS instance with the C/C++ build tools.
///
/// vswhere is the official Microsoft tool for VS install discovery.
/// It ships at a fixed path inside the VS Installer and works for every
/// VS edition + year combination — replacing the year/SKU enumeration
/// that would otherwise need updating with each new VS release.
fn vswhere_vc_tools_include() -> Option<PathBuf> {
    let vswhere = locate_vswhere()?;
    let output = std::process::Command::new(&vswhere)
        .args([
            "-latest",
            "-products", "*",
            "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property", "installationPath",
            "-format", "value",
            "-utf8",
        ])
        .output()
        .ok()?;
    if !output.status.success() { return None }
    let install_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if install_path.is_empty() { return None }
    let msvc_dir = PathBuf::from(install_path).join("VC").join("Tools").join("MSVC");
    let version_dir = newest_subdir(&msvc_dir)?;
    let include = version_dir.join("include");
    if include.is_dir() { Some(include) } else { None }
}

/// vswhere is installed at `<ProgramFiles(x86)>\Microsoft Visual Studio\
/// Installer\vswhere.exe` on every machine that has the VS Installer
/// (released alongside VS 2017). Probe the standard locations and
/// `VSINSTALLDIR` env var for completeness.
fn locate_vswhere() -> Option<PathBuf> {
    if let Some(env) = std::env::var_os("VSINSTALLDIR") {
        // VSINSTALLDIR points at a VS instance, not the installer; walk
        // up to the installer directory.
        if let Some(parent) = PathBuf::from(env).parent().and_then(|p| p.parent()) {
            let candidate = parent.join("Installer").join("vswhere.exe");
            if candidate.is_file() { return Some(candidate) }
        }
    }
    for base in [
        std::env::var_os("ProgramFiles(x86)"),
        std::env::var_os("ProgramFiles"),
    ].into_iter().flatten() {
        let p = PathBuf::from(base)
            .join("Microsoft Visual Studio")
            .join("Installer")
            .join("vswhere.exe");
        if p.is_file() { return Some(p) }
    }
    None
}

/// Generic walk of `<base>/<year-dir>/<edition-dir>/VC/Tools/MSVC` for
/// any subdirectory shape — no hardcoded year or edition lists. Picks
/// the lexicographically-greatest year, then the
/// lexicographically-greatest edition with a populated MSVC dir.
fn discover_vc_tools_include_layout(bases: &[PathBuf]) -> Option<PathBuf> {
    for base in bases {
        // Iterate every immediate subdir as a candidate year. Preferring
        // the lexicographically-greatest one mirrors "newest year".
        let mut years = list_subdirs(base);
        years.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
        for year_dir in years.into_iter().rev() {
            let mut editions = list_subdirs(&year_dir);
            editions.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
            for edition_dir in editions.into_iter().rev() {
                let msvc_dir = edition_dir.join("VC").join("Tools").join("MSVC");
                let Some(version_dir) = newest_subdir(&msvc_dir) else { continue };
                let include = version_dir.join("include");
                if include.is_dir() { return Some(include); }
            }
        }
    }
    None
}

fn list_subdirs(parent: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(parent)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect()
}

/// Return the lexicographically-newest immediate subdirectory of
/// `parent`, or `None` if `parent` doesn't exist or has no subdirs.
fn newest_subdir(parent: &Path) -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(parent)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    entries.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    entries.into_iter().next_back()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "msvc_sdk_tests.rs"]
mod tests;
