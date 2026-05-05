// =============================================================================
// ecosystem/flutter_sdk.rs — Flutter SDK stdlib ecosystem
//
// Indexes the Flutter framework source so that flutter: imports (widgets,
// material, cupertino, etc.) resolve against real symbol definitions rather
// than being recorded as unresolved refs.
//
// Probe order:
//   1. BEARWISDOM_FLUTTER_SDK env var (direct path to packages/flutter/)
//   2. FLUTTER_ROOT env var
//   3. `flutter` binary on PATH → walk up to sdk root
//   4. Well-known paths: ~/flutter, /opt/flutter, /usr/local/flutter,
//      Windows: C:/src/flutter
//
// Walked subtrees:
//   packages/flutter/lib/src/     — implementation
//   packages/flutter/lib/*.dart   — barrel files
//   packages/flutter_test/lib/    — if present
//   packages/flutter_localizations/lib/ — if present
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;

use tracing::debug;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("flutter-sdk");
const LEGACY_ECOSYSTEM_TAG: &str = "flutter-sdk";
const LANGUAGES: &[&str] = &["dart"];

/// Extra Flutter sub-packages to index alongside the core flutter package.
const EXTRA_FLUTTER_PACKAGES: &[&str] = &["flutter_test", "flutter_localizations"];

pub struct FlutterSdkEcosystem;

impl Ecosystem for FlutterSdkEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        // A Dart project is a Flutter project iff its pubspec.yaml lists
        // a `flutter` dep — usually `flutter: { sdk: flutter }`. Pure
        // Dart projects (CLI tools, server packages, packages with no
        // Flutter dep) do not activate this ecosystem; they get
        // dart_sdk only.
        EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/pubspec.yaml",
            field_path: "dependencies",
            value: "flutter",
        }
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_flutter_sdk()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_flutter_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        super::dart_sdk::build_dart_symbol_index(dep_roots)
    }

    /// Pre-pull every `.dart` file under each Flutter root. Flutter is
    /// demand-driven but bare type refs (`IconData`,
    /// `RoundedRectangleBorder`, `EdgeInsets`, `FocusNode`) don't trigger
    /// chain-miss expansion, so without an eager pull the resolver's
    /// simple-name lookup never sees Flutter framework types and they
    /// stay unresolved despite the SDK being discovered. `walk_flutter_
    /// root` is the same walk used by the legacy eager path — it skips
    /// `test/`/`tests/` and dotted dirs, bounded by `MAX_WALK_DEPTH`.
    ///
    /// Cost: ~700 files across `flutter`/`flutter_test`/
    /// `flutter_localizations` libs. Well under the walker budget; the
    /// indexer's parallel pipeline parses them in a few seconds.
    fn demand_pre_pull(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> Vec<WalkedFile> {
        let mut out = Vec::new();
        for dep in dep_roots {
            out.extend(walk_flutter_root(dep));
        }
        out
    }
}

impl ExternalSourceLocator for FlutterSdkEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_flutter_sdk()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_flutter_root(dep)
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_flutter_sdk() -> Vec<ExternalDepRoot> {
    let Some(flutter_root) = probe_flutter_root() else {
        debug!("flutter-sdk: no Flutter SDK probe succeeded");
        return Vec::new();
    };
    debug!("flutter-sdk: using {}", flutter_root.display());

    let mut roots = Vec::new();

    // Core flutter package
    let flutter_lib = flutter_root.join("packages").join("flutter").join("lib");
    if flutter_lib.is_dir() {
        roots.push(ExternalDepRoot {
            module_path: "flutter".to_string(),
            version: String::new(),
            root: flutter_lib,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }

    // Optional extra packages
    for pkg in EXTRA_FLUTTER_PACKAGES {
        let pkg_lib = flutter_root.join("packages").join(pkg).join("lib");
        if pkg_lib.is_dir() {
            roots.push(ExternalDepRoot {
                module_path: pkg.to_string(),
                version: String::new(),
                root: pkg_lib,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
        }
    }

    if roots.is_empty() {
        debug!(
            "flutter-sdk: found flutter root at {} but packages/flutter/lib/ absent",
            flutter_root.display()
        );
    }
    roots
}

fn probe_flutter_root() -> Option<PathBuf> {
    // 1. Explicit override pointing directly at packages/flutter/
    if let Some(raw) = std::env::var_os("BEARWISDOM_FLUTTER_SDK") {
        let p = PathBuf::from(raw);
        // Accept both: full flutter root or direct packages/flutter/ path
        if p.join("lib").is_dir() {
            // Caller gave packages/flutter/ directly
            return p
                .parent()?
                .parent()
                .map(|r| r.to_path_buf())
                .filter(|r| r.join("packages").join("flutter").is_dir());
        }
        if p.join("packages").join("flutter").is_dir() {
            return Some(p);
        }
    }

    // 2. FLUTTER_ROOT env var
    if let Some(raw) = std::env::var_os("FLUTTER_ROOT") {
        let p = PathBuf::from(raw);
        if p.join("packages").join("flutter").is_dir() {
            return Some(p);
        }
    }

    // 3. `flutter` binary on PATH
    if let Some(root) = flutter_bin_root("flutter") {
        if root.join("packages").join("flutter").is_dir() {
            return Some(root);
        }
    }

    // 4. Well-known paths
    for candidate in well_known_flutter_paths() {
        if candidate.join("packages").join("flutter").is_dir() {
            return Some(candidate);
        }
    }

    None
}

fn flutter_bin_root(bin: &str) -> Option<PathBuf> {
    let which_cmd = if cfg!(windows) { "where" } else { "which" };
    let Ok(output) = Command::new(which_cmd).arg(bin).output() else {
        return None;
    };
    if !output.status.success() { return None; }
    let s = String::from_utf8(output.stdout).ok()?;
    let binary_path = PathBuf::from(s.lines().next()?.trim());
    // <flutter_root>/bin/flutter → parent = <flutter_root>/bin → parent = <flutter_root>
    let resolved = binary_path.canonicalize().unwrap_or(binary_path);
    resolved.parent()?.parent().map(|p| p.to_path_buf())
}

fn well_known_flutter_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    if let Some(home) = home {
        out.push(PathBuf::from(&home).join("flutter"));
        out.push(PathBuf::from(&home).join("snap").join("flutter").join("common").join("flutter"));
    }
    out.push(PathBuf::from("/opt/flutter"));
    out.push(PathBuf::from("/usr/local/flutter"));
    out.push(PathBuf::from("/usr/lib/flutter"));
    if cfg!(windows) {
        out.push(PathBuf::from("C:/src/flutter"));
        out.push(PathBuf::from("C:/flutter"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            out.push(PathBuf::from(local).join("flutter"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_flutter_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    // Walk lib/src/ recursively
    let src_dir = dep.root.join("src");
    if src_dir.is_dir() {
        walk_sdk_dir(&src_dir, &dep.root, dep, &mut out, 0);
    }
    // Barrel files at lib/*.dart
    let Ok(entries) = std::fs::read_dir(&dep.root) else { return out; };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue; };
        if !ft.is_file() { continue; }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
        if !name.ends_with(".dart") { continue; }
        let rel = match path.strip_prefix(&dep.root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        out.push(WalkedFile {
            relative_path: format!("ext:flutter-sdk:{}/{}", dep.module_path, rel),
            absolute_path: path,
            language: "dart",
        });
    }
    out
}

fn walk_sdk_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= 8 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue; };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') { continue; }
                if matches!(name, "test" | "tests") { continue; }
            }
            walk_sdk_dir(&path, root, dep, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !name.ends_with(".dart") { continue; }
            let rel = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:flutter-sdk:{}/{}", dep.module_path, rel),
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
#[path = "flutter_sdk_tests.rs"]
mod tests;
