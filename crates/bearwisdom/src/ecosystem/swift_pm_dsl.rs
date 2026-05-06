// =============================================================================
// ecosystem/swift_pm_dsl.rs — Swift Package Manager DSL types
//
// `Package.swift` manifests reference DSL types — `Package`, `Target`,
// `Product`, `.target(name:dependencies:)`, `.executableTarget(...)`,
// `.package(url:from:)` — that don't live anywhere in the project tree.
// They ship inside the Swift toolchain at
// `<toolchain>/usr/lib/swift/pm/ManifestAPI/PackageDescription.swiftinterface`.
//
// `.swiftinterface` is a text-format Swift module interface (a curated
// subset of Swift declarations); tree-sitter-swift parses it the same as
// regular `.swift` source. This walker emits the ManifestAPI files as
// `language: "swift"` walked files so the standard Swift extractor produces
// the symbols.
//
// Toolchain probe order:
//   1. $BEARWISDOM_SWIFT_TOOLCHAIN — explicit toolchain root override.
//   2. `xcrun --find swiftc` (macOS) → Xcode-default toolchain.
//   3. Common system install locations on each OS.
//
// Activation: project's `Package.swift` contains `import PackageDescription`
// (canonical first line). Pure Xcode-app projects without SPM don't pay
// the toolchain probe.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("swift-pm-dsl");
const TAG: &str = "swift-pm-dsl";
const LANGUAGES: &[&str] = &["swift"];

// ---------------------------------------------------------------------------
// Ecosystem impl
// ---------------------------------------------------------------------------

pub struct SwiftPmDslEcosystem;

impl Ecosystem for SwiftPmDslEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/Package.swift",
            field_path: "",
            value: "import PackageDescription",
        }
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_manifest_api()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_manifest_api(dep)
    }

    fn supports_reachability(&self) -> bool { true }
    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for SwiftPmDslEcosystem {
    fn ecosystem(&self) -> &'static str { TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_manifest_api()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_manifest_api(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<SwiftPmDslEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(SwiftPmDslEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Toolchain discovery
// ---------------------------------------------------------------------------

fn discover_manifest_api() -> Vec<ExternalDepRoot> {
    let Some(manifest_dir) = probe_manifest_api_dir() else {
        debug!("swift-pm-dsl: no toolchain ManifestAPI dir probed");
        return Vec::new();
    };
    debug!("swift-pm-dsl: using {}", manifest_dir.display());
    vec![ExternalDepRoot {
        module_path: "PackageDescription".to_string(),
        version: String::new(),
        root: manifest_dir,
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn probe_manifest_api_dir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_SWIFT_TOOLCHAIN") {
        let toolchain = PathBuf::from(explicit);
        if let Some(p) = manifest_api_under(&toolchain) { return Some(p); }
    }
    if let Some(p) = probe_via_xcrun() {
        if let Some(found) = manifest_api_under(&p) { return Some(found); }
    }
    for candidate in standard_toolchain_paths() {
        if let Some(found) = manifest_api_under(&candidate) {
            return Some(found);
        }
    }
    None
}

fn standard_toolchain_paths() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    // macOS — Xcode's default toolchain plus standalone swift.org installs.
    out.push(PathBuf::from(
        "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain",
    ));
    out.push(PathBuf::from(
        "/Library/Developer/CommandLineTools",
    ));
    if let Some(home) = dirs::home_dir() {
        out.push(home.join("Library/Developer/Toolchains/swift-latest.xctoolchain"));
    }
    // Linux — swift.org tarball default install + apt/dnf paths.
    out.push(PathBuf::from("/usr/share/swift"));
    out.push(PathBuf::from("/usr/lib/swift"));
    out.push(PathBuf::from("/opt/swift"));
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".swiftly").join("toolchains").join("latest"));
    }
    // Windows — swift.org installer (winget / official msi) and chocolatey.
    out.push(PathBuf::from(
        "C:/Library/Developer/Toolchains/unknown-Asserts-development.xctoolchain",
    ));
    out.push(PathBuf::from("C:/Program Files/Swift"));
    out
}

/// Translate a toolchain root candidate into the `ManifestAPI` dir if it
/// exists. Tries the canonical layout first, then a couple of legacy
/// shapes.
fn manifest_api_under(toolchain: &Path) -> Option<PathBuf> {
    if !toolchain.is_dir() { return None }
    // Modern: <toolchain>/usr/lib/swift/pm/ManifestAPI
    let modern = toolchain.join("usr/lib/swift/pm/ManifestAPI");
    if modern.is_dir() { return Some(modern); }
    // Some Linux distros: <toolchain>/lib/swift/pm/ManifestAPI
    let alt = toolchain.join("lib/swift/pm/ManifestAPI");
    if alt.is_dir() { return Some(alt); }
    // Windows swift.org installer: <toolchain>/usr/lib/swift_static/pm/ManifestAPI
    let win = toolchain.join("usr/lib/swift_static/pm/ManifestAPI");
    if win.is_dir() { return Some(win); }
    None
}

fn probe_via_xcrun() -> Option<PathBuf> {
    // `xcrun --find swiftc` returns the path to the swiftc binary inside the
    // active toolchain. The toolchain root is the bin/ ancestor.
    let output = Command::new("xcrun").args(["--find", "swiftc"]).output().ok()?;
    if !output.status.success() { return None; }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let swiftc = PathBuf::from(stdout.trim());
    // <toolchain>/usr/bin/swiftc → toolchain root is parent.parent.parent
    swiftc.parent()?.parent()?.parent().map(|p| p.to_path_buf())
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_manifest_api(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dep.root) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() { continue }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        // `.swiftinterface` is the textual module interface; some toolchains
        // also ship a redundant `.private.swiftinterface`. Index the public
        // surface only — the private one duplicates declarations and adds
        // ambiguity to lookups.
        if !name.ends_with(".swiftinterface") || name.ends_with(".private.swiftinterface") {
            continue;
        }
        let display = path.to_string_lossy().replace('\\', "/");
        out.push(WalkedFile {
            relative_path: format!("ext:swift-pm-dsl:{name}"),
            absolute_path: path,
            language: "swift",
        });
        let _ = display;
    }
    out
}

#[cfg(test)]
#[path = "swift_pm_dsl_tests.rs"]
mod tests;
