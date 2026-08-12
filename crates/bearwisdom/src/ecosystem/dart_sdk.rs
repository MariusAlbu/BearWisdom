// =============================================================================
// ecosystem/dart_sdk.rs — Dart SDK stdlib ecosystem
//
// Probes the Dart SDK lib/ directory and indexes the core stdlib packages:
// core, async, collection, convert, io, isolate, math, typed_data,
// developer, ffi.
//
// Probe order:
//   1. BEARWISDOM_DART_SDK env var override
//   2. DART_SDK env var
//   3. FLUTTER_ROOT/bin/cache/dart-sdk/
//   4. `dart` binary on PATH → walk up to find sdk root
//   5. Well-known install paths (Windows: Program Files/Dart/dart-sdk,
//      macOS: /usr/lib/dart, Linux: /usr/lib/dart)
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;

use tracing::debug;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::walker::WalkedFile;

#[path = "dart_sdk_symbol_index.rs"]
mod dart_sdk_symbol_index;
pub(super) use dart_sdk_symbol_index::build_dart_symbol_index;

pub const ID: EcosystemId = EcosystemId::new("dart-sdk");
/// `dart-sdk` ecosystem tag. Also stamped on `FlutterSdkEcosystem`'s
/// sky_engine `ui` dep root — `dart:ui` is a `dart:` import shipped by
/// Flutter's bundled sky_engine instead of the plain Dart SDK, so it shares
/// this scheme rather than getting a second `ext:` identity.
pub(crate) const LEGACY_ECOSYSTEM_TAG: &str = "dart-sdk";
const LANGUAGES: &[&str] = &["dart"];

/// Sub-libraries of `lib/` that constitute the public Dart SDK stdlib.
pub(crate) const DART_SDK_LIBS: &[&str] = &[
    "core",
    "async",
    "collection",
    "convert",
    "io",
    "isolate",
    // `dart:ui` sources ship in sky_engine (Flutter engine bindings), walked
    // under the same `ext:dart-sdk:ui/` identity; a real Dart SDK has no
    // `lib/ui`, so the walker probe simply misses there.
    "ui",
    "math",
    "typed_data",
    "developer",
    "ffi",
];

pub struct DartSdkEcosystem;

impl Ecosystem for DartSdkEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Stdlib
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("dart")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_dart_sdk()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_dart_sdk(dep)
    }

    fn supports_reachability(&self) -> bool {
        true
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_dart_symbol_index(dep_roots)
    }
}

impl ExternalSourceLocator for DartSdkEcosystem {
    fn ecosystem(&self) -> &'static str {
        LEGACY_ECOSYSTEM_TAG
    }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_dart_sdk()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_dart_sdk(dep)
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_dart_sdk() -> Vec<ExternalDepRoot> {
    let Some(lib_dir) = probe_dart_sdk_lib() else {
        debug!("dart-sdk: no SDK probe succeeded");
        return Vec::new();
    };
    debug!("dart-sdk: using {}", lib_dir.display());
    vec![ExternalDepRoot {
        module_path: "dart-sdk".to_string(),
        version: String::new(),
        root: lib_dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn probe_dart_sdk_lib() -> Option<PathBuf> {
    // 1. Explicit override
    if let Some(raw) = std::env::var_os("BEARWISDOM_DART_SDK") {
        let p = PathBuf::from(raw).join("lib");
        if p.is_dir() {
            return Some(p);
        }
    }

    // 2. DART_SDK env var (points to sdk root, not lib/)
    if let Some(raw) = std::env::var_os("DART_SDK") {
        let p = PathBuf::from(raw).join("lib");
        if p.is_dir() {
            return Some(p);
        }
    }

    // 3. FLUTTER_ROOT bundled dart-sdk
    if let Some(raw) = std::env::var_os("FLUTTER_ROOT") {
        let p = PathBuf::from(raw)
            .join("bin")
            .join("cache")
            .join("dart-sdk")
            .join("lib");
        if p.is_dir() {
            return Some(p);
        }
    }

    // 4. `dart` binary on PATH → resolve symlinks and walk up
    if let Some(sdk_root) = dart_bin_sdk_root("dart") {
        let p = sdk_root.join("lib");
        if p.is_dir() {
            return Some(p);
        }
    }

    // 5. Well-known install paths
    for candidate in well_known_dart_sdk_paths() {
        let p = candidate.join("lib");
        if p.is_dir() {
            return Some(p);
        }
    }

    None
}

/// Invoke `dart --print-sdk-directory` or locate via PATH to find sdk root.
/// The dart binary lives at `<sdk>/bin/dart`.
fn dart_bin_sdk_root(bin: &str) -> Option<PathBuf> {
    // Try `dart --print-sdk-directory` first (Dart 2.x+)
    if let Ok(output) = Command::new(bin).arg("--print-sdk-directory").output() {
        if output.status.success() {
            let s = String::from_utf8(output.stdout).ok()?;
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                let p = PathBuf::from(trimmed);
                if p.is_dir() {
                    return Some(p);
                }
            }
        }
    }

    // Fallback: locate the binary and walk up
    let which_cmd = if cfg!(windows) { "where" } else { "which" };
    let Ok(output) = Command::new(which_cmd).arg(bin).output() else {
        return None;
    };
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8(output.stdout).ok()?;
    let binary_path = PathBuf::from(s.lines().next()?.trim());
    let resolved = binary_path.canonicalize().unwrap_or(binary_path);
    // <sdk>/bin/dart → parent = <sdk>/bin → parent = <sdk>
    resolved.parent()?.parent().map(|p| p.to_path_buf())
}

fn well_known_dart_sdk_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(windows) {
        out.push(PathBuf::from("C:/Program Files/Dart/dart-sdk"));
        out.push(PathBuf::from("C:/tools/dart-sdk"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            out.push(PathBuf::from(local).join("Pub").join("dart-sdk"));
        }
    }
    out.push(PathBuf::from("/usr/lib/dart"));
    out.push(PathBuf::from("/usr/local/lib/dart"));
    out.push(PathBuf::from("/opt/dart-sdk"));
    out.push(PathBuf::from("/opt/homebrew/opt/dart/libexec"));
    out
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_dart_sdk(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    for lib_name in DART_SDK_LIBS {
        let sub = dep.root.join(lib_name);
        if sub.is_dir() {
            walk_sdk_dir(&sub, &dep.root, dep, &mut out, 0);
        }
    }
    out
}

/// Walk `dir` recursively, emitting `ext:dart-sdk:<rel>` `WalkedFile`s with
/// `rel` relative to `root`. Shared by `DartSdkEcosystem` (its own SDK
/// `lib/` as both `dir` and `root`) and `FlutterSdkEcosystem` (sky_engine's
/// `lib/ui/` as `dir`, sky_engine's `lib/` as `root`, so `rel` keeps the
/// `ui/` segment and lands under the same scheme).
pub(crate) fn walk_sdk_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') {
                    continue;
                }
                if matches!(name, "test" | "tests") {
                    continue;
                }
            }
            walk_sdk_dir(&path, root, dep, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".dart") {
                continue;
            }
            let rel = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:dart-sdk:{}", rel),
                absolute_path: path,
                language: "dart",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "dart_sdk_tests.rs"]
mod tests;
