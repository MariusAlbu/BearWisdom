// =============================================================================
// ecosystem/posix_headers.rs — POSIX + MSVC C/C++ headers (stdlib ecosystem)
//
// Two ecosystems covering platform C/C++ headers:
//   * PosixHeadersEcosystem — /usr/include on unix-like systems.
//   * MsvcHeadersEcosystem  — $VCINSTALLDIR/include on Windows.
//
// Both gate on `LanguagePresent("c") OR LanguagePresent("cpp")` plus the
// platform check. On the wrong platform the ecosystem's activation
// returns false and nothing probes.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
    Platform,
};
use crate::indexer::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

// ---------------------------------------------------------------------------
// PosixHeadersEcosystem
// ---------------------------------------------------------------------------

pub const POSIX_ID: EcosystemId = EcosystemId::new("posix-headers");
const POSIX_TAG: &str = "posix-headers";

pub struct PosixHeadersEcosystem;

impl Ecosystem for PosixHeadersEcosystem {
    fn id(&self) -> EcosystemId { POSIX_ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { &["c", "cpp"] }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::All(&[
            EcosystemActivation::AlwaysOnPlatform(Platform::Unix),
            EcosystemActivation::Any(&[
                EcosystemActivation::LanguagePresent("c"),
                EcosystemActivation::LanguagePresent("cpp"),
            ]),
        ])
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_posix_include()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_headers(dep, POSIX_TAG)
    }
}

impl ExternalSourceLocator for PosixHeadersEcosystem {
    fn ecosystem(&self) -> &'static str { POSIX_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_posix_include()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_headers(dep, POSIX_TAG)
    }
}

fn discover_posix_include() -> Vec<ExternalDepRoot> {
    if cfg!(target_os = "windows") { return Vec::new() }
    let mut out = Vec::new();
    let candidates = [
        "/usr/include",
        "/usr/local/include",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_dir() {
            out.push(make_root(&p, POSIX_TAG));
        }
    }
    if let Some(explicit) = std::env::var_os("BEARWISDOM_POSIX_INCLUDE") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            out.push(make_root(&p, POSIX_TAG));
        }
    }
    if out.is_empty() {
        debug!("posix-headers: no /usr/include found");
    }
    out
}

pub fn posix_shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PosixHeadersEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(PosixHeadersEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// MsvcHeadersEcosystem
// ---------------------------------------------------------------------------

pub const MSVC_ID: EcosystemId = EcosystemId::new("msvc-headers");
const MSVC_TAG: &str = "msvc-headers";

pub struct MsvcHeadersEcosystem;

impl Ecosystem for MsvcHeadersEcosystem {
    fn id(&self) -> EcosystemId { MSVC_ID }
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

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_msvc_include()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_headers(dep, MSVC_TAG)
    }
}

impl ExternalSourceLocator for MsvcHeadersEcosystem {
    fn ecosystem(&self) -> &'static str { MSVC_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_msvc_include()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_headers(dep, MSVC_TAG)
    }
}

fn discover_msvc_include() -> Vec<ExternalDepRoot> {
    if !cfg!(target_os = "windows") { return Vec::new() }
    let mut out = Vec::new();
    if let Some(vc) = std::env::var_os("VCINSTALLDIR") {
        let p = PathBuf::from(vc).join("include");
        if p.is_dir() {
            out.push(make_root(&p, MSVC_TAG));
        }
    }
    if let Some(wdk) = std::env::var_os("WindowsSdkDir") {
        let p = PathBuf::from(wdk).join("Include");
        if p.is_dir() {
            out.push(make_root(&p, MSVC_TAG));
        }
    }
    if let Some(explicit) = std::env::var_os("BEARWISDOM_MSVC_INCLUDE") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            out.push(make_root(&p, MSVC_TAG));
        }
    }
    if out.is_empty() {
        debug!("msvc-headers: no VCINSTALLDIR / WindowsSdkDir / override probed");
    }
    out
}

pub fn msvc_shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<MsvcHeadersEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(MsvcHeadersEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn make_root(dir: &Path, tag: &'static str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: dir.to_string_lossy().into_owned(),
        version: String::new(),
        root: dir.to_path_buf(),
        ecosystem: tag,
        package_id: None,
    }
}

fn walk_headers(dep: &ExternalDepRoot, _tag: &'static str) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= 10 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Skip noisy sub-dirs but keep the main platform headers.
                if matches!(name, "tests" | "test") { continue }
            }
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let lang = match () {
                _ if name.ends_with(".h") => "c",
                _ if name.ends_with(".hpp") || name.ends_with(".hxx") || name.ends_with(".hh") => "cpp",
                _ => continue,
            };
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:{lang}:{}", display),
                absolute_path: path,
                language: lang,
            });
        }
    }
}
