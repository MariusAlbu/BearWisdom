// =============================================================================
// ecosystem/php_stubs.rs — JetBrains phpstorm-stubs (stdlib ecosystem)
//
// PHP's standard library + every common PECL extension are described as
// regular PHP source files in the open-source JetBrains/phpstorm-stubs
// repo. Users point us at a local checkout via $BEARWISDOM_PHP_STUBS_DIR
// or a conventional location. We walk the .php files and let the PHP
// plugin index them like any other source.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("php-stubs");
const LEGACY_ECOSYSTEM_TAG: &str = "php-stubs";
const LANGUAGES: &[&str] = &["php"];

pub struct PhpStubsEcosystem;

impl Ecosystem for PhpStubsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("php")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_php_stubs()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        let mut out = Vec::new();
        walk_dir(&dep.root, &mut out, 0);
        out
    }
}

impl ExternalSourceLocator for PhpStubsEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_php_stubs()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        let mut out = Vec::new();
        walk_dir(&dep.root, &mut out, 0);
        out
    }
}

fn discover_php_stubs() -> Vec<ExternalDepRoot> {
    let Some(dir) = probe_stubs_dir() else {
        debug!("php-stubs: no stubs checkout found; set $BEARWISDOM_PHP_STUBS_DIR");
        return Vec::new();
    };
    vec![ExternalDepRoot {
        module_path: "phpstorm-stubs".to_string(),
        version: String::new(),
        root: dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
    }]
}

fn probe_stubs_dir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_PHP_STUBS_DIR") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p); }
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        for sub in ["phpstorm-stubs", ".phpstorm-stubs", "dev/phpstorm-stubs"] {
            let p = PathBuf::from(&home).join(sub);
            if p.is_dir() { return Some(p); }
        }
    }
    None
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= 10 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "test" | ".git" | "meta" | "Examples") { continue }
                if name.starts_with('.') { continue }
            }
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".php") { continue }
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:php:{}", display),
                absolute_path: path,
                language: "php",
            });
        }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PhpStubsEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(PhpStubsEcosystem)).clone()
}
