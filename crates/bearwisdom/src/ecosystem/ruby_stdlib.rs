// =============================================================================
// ecosystem/ruby_stdlib.rs — Ruby stdlib (stdlib ecosystem)
//
// Probes `RbConfig::CONFIG["rubylibdir"]` via the system `ruby` binary
// or a well-known install dir. Walks top-level .rb files for the
// standard library (net/, json/, etc.).
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

pub const ID: EcosystemId = EcosystemId::new("ruby-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "ruby-stdlib";
const LANGUAGES: &[&str] = &["ruby"];

pub struct RubyStdlibEcosystem;

impl Ecosystem for RubyStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("ruby")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_ruby_stdlib()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ruby_tree(dep)
    }
}

impl ExternalSourceLocator for RubyStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_ruby_stdlib()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ruby_tree(dep)
    }
}

fn discover_ruby_stdlib() -> Vec<ExternalDepRoot> {
    let Some(dir) = probe_rubylibdir() else {
        debug!("ruby-stdlib: no rubylibdir probe");
        return Vec::new();
    };
    debug!("ruby-stdlib: using {}", dir.display());
    vec![ExternalDepRoot {
        module_path: "ruby-stdlib".to_string(),
        version: String::new(),
        root: dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
    }]
}

fn probe_rubylibdir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_RUBY_STDLIB") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p); }
    }
    for bin in ["ruby"] {
        let Ok(output) = Command::new(bin)
            .args(["-e", "print RbConfig::CONFIG['rubylibdir']"])
            .output()
        else {
            continue;
        };
        if !output.status.success() { continue }
        let s = String::from_utf8(output.stdout).ok()?;
        let trimmed = s.trim();
        if trimmed.is_empty() { continue }
        let p = PathBuf::from(trimmed);
        if p.is_dir() { return Some(p); }
    }
    None
}

fn walk_ruby_tree(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= 12 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "spec") { continue }
                if name.starts_with('.') { continue }
            }
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".rb") { continue }
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:ruby:{}", display),
                absolute_path: path,
                language: "ruby",
            });
        }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<RubyStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(RubyStdlibEcosystem)).clone()
}
