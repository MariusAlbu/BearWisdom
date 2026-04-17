// =============================================================================
// ecosystem/rust_stdlib.rs — Rust stdlib (stdlib ecosystem)
//
// Probes `rustc --print=sysroot`, locates
// `{sysroot}/lib/rustlib/src/rust/library/{std,core,alloc,...}/`, and
// walks those crates as plain Rust source. Requires the `rust-src`
// rustup component (`rustup component add rust-src`) — if missing, the
// ecosystem degrades to an empty discovery silently.
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

pub const ID: EcosystemId = EcosystemId::new("rust-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "rust-stdlib";
const LANGUAGES: &[&str] = &["rust"];

/// The crates we care about — the `rust-src` component ships many more
/// (test, panic_abort, panic_unwind, profiler_builtins, ...) but they
/// produce noise in completion results. Extend this list if a specific
/// crate's symbols are needed.
const STDLIB_CRATES: &[&str] = &["std", "core", "alloc", "proc_macro"];

pub struct RustStdlibEcosystem;

impl Ecosystem for RustStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("rust")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_rust_stdlib_roots()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_rust_tree(dep)
    }
}

impl ExternalSourceLocator for RustStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_rust_stdlib_roots()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_rust_tree(dep)
    }
}

fn discover_rust_stdlib_roots() -> Vec<ExternalDepRoot> {
    let Some(sysroot) = rustc_sysroot() else { return Vec::new() };
    let library_dir = sysroot.join("lib").join("rustlib").join("src").join("rust").join("library");
    if !library_dir.is_dir() {
        debug!(
            "rust-stdlib: source tree not found at {}; install `rustup component add rust-src`",
            library_dir.display()
        );
        return Vec::new();
    }
    let mut roots = Vec::new();
    for crate_name in STDLIB_CRATES {
        let crate_dir = library_dir.join(crate_name).join("src");
        if !crate_dir.is_dir() { continue }
        roots.push(ExternalDepRoot {
            module_path: crate_name.to_string(),
            version: String::new(),
            root: crate_dir,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
        });
    }
    roots
}

fn rustc_sysroot() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_RUST_SYSROOT") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p) }
    }
    // Try rustc.
    let output = Command::new("rustc")
        .arg("--print=sysroot")
        .output()
        .ok()?;
    if !output.status.success() { return None }
    let path = String::from_utf8(output.stdout).ok()?;
    let trimmed = path.trim();
    if trimmed.is_empty() { return None }
    let p = PathBuf::from(trimmed);
    if p.is_dir() { Some(p) } else { None }
}

fn walk_rust_tree(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= 16 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "benches" | "examples") { continue }
                if name.starts_with('.') { continue }
            }
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".rs") { continue }
            if name == "lib.rs" || name == "mod.rs" || !name.starts_with("tests") {
                let display = path.to_string_lossy().replace('\\', "/");
                let rel = format!("ext:rust:{}", display);
                out.push(WalkedFile {
                    relative_path: rel,
                    absolute_path: path,
                    language: "rust",
                });
            }
        }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<RustStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(RustStdlibEcosystem)).clone()
}
