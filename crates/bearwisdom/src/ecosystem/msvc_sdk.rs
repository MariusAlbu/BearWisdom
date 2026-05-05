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
        // Precedence: when compile_commands.json is present its -I paths
        // are ground truth — no heuristic SDK probe.
        if super::compile_commands::project_has_compile_commands_json(ctx.project_root) {
            return Vec::new();
        }

        // Project gate: scan for *.vcxproj. When none are found, the
        // project is not building with MSBuild and the Windows SDK is
        // not the right include source — return empty.
        let vcxprojs = find_vcxproj_files(ctx.project_root);
        if vcxprojs.is_empty() {
            debug!("msvc-sdk: no *.vcxproj in project; skipping SDK probe");
            return Vec::new();
        }

        let pinned_version = pinned_target_platform_version(&vcxprojs);
        discover_msvc_include(pinned_version.as_deref())
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn supports_reachability(&self) -> bool { true }
    fn uses_demand_driven_parse(&self) -> bool { true }

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
        if vcxprojs.is_empty() {
            return Vec::new();
        }
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
    if let Some(wdk) = std::env::var_os("WindowsSdkDir") {
        let include_root = PathBuf::from(wdk).join("Include");
        if include_root.is_dir() {
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "msvc_sdk_tests.rs"]
mod tests;
