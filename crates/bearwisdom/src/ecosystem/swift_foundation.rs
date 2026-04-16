// =============================================================================
// ecosystem/swift_foundation.rs — Swift Foundation + stdlib (stdlib ecosystem)
//
// Probes an Xcode SDK (on macOS) or a Swift toolchain's usr/lib/swift/
// dir for `.swiftinterface` files. These describe Foundation, Swift,
// Dispatch, and Combine in plain Swift — the BearWisdom Swift plugin
// can index them like any other .swift source.
//
// On non-Darwin hosts the open-source Swift toolchain ships the same
// layout under `$SWIFT_ROOT/usr/lib/swift/`.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::indexer::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("swift-foundation");
const LEGACY_ECOSYSTEM_TAG: &str = "swift-foundation";
const LANGUAGES: &[&str] = &["swift"];

pub struct SwiftFoundationEcosystem;

impl Ecosystem for SwiftFoundationEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("swift")
    }
    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> { discover() }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk(dep) }
}

impl ExternalSourceLocator for SwiftFoundationEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> { discover() }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk(dep) }
}

fn discover() -> Vec<ExternalDepRoot> {
    let Some(dir) = probe_swift_lib() else {
        debug!("swift-foundation: no Swift stdlib interface dir probed");
        return Vec::new();
    };
    vec![ExternalDepRoot {
        module_path: "swift-stdlib".to_string(),
        version: String::new(),
        root: dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
    }]
}

fn probe_swift_lib() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_SWIFT_STDLIB") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p); }
    }
    // Xcode SDK.
    if let Ok(output) = Command::new("xcode-select").arg("-p").output() {
        if output.status.success() {
            let dev = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
            let sdk = dev
                .join("Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk/usr/lib/swift");
            if sdk.is_dir() { return Some(sdk); }
        }
    }
    // Open-source Swift toolchain.
    if let Some(root) = std::env::var_os("SWIFT_ROOT") {
        let p = PathBuf::from(root).join("usr").join("lib").join("swift");
        if p.is_dir() { return Some(p); }
    }
    for candidate in [
        "/usr/lib/swift",
        "/usr/local/lib/swift",
        "/Library/Developer/Toolchains/swift-latest.xctoolchain/usr/lib/swift",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_dir() { return Some(p); }
    }
    None
}

fn walk(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= 6 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            // .swiftinterface is a valid Swift source flavor; our plugin
            // parses it as .swift because the grammar accepts the subset.
            if !(name.ends_with(".swiftinterface") || name.ends_with(".swift")) { continue }
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:swift:{}", display),
                absolute_path: path,
                language: "swift",
            });
        }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<SwiftFoundationEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(SwiftFoundationEcosystem)).clone()
}
