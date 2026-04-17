// =============================================================================
// ecosystem/groovy_stdlib.rs — Groovy stdlib (stdlib ecosystem)
//
// Probes `$GROOVY_HOME/lib/groovy-*-sources.jar` and extracts it through
// the shared Maven sources-jar helper. Activation:
// LanguagePresent("groovy").
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{
    extract_java_sources_jar, is_cache_stale, ExternalDepRoot, ExternalSourceLocator,
};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("groovy-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "groovy-stdlib";
const LANGUAGES: &[&str] = &["groovy"];

pub struct GroovyStdlibEcosystem;

impl Ecosystem for GroovyStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("groovy")
    }
    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> { discover() }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }
}

impl ExternalSourceLocator for GroovyStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> { discover() }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }
}

fn discover() -> Vec<ExternalDepRoot> {
    let Some(home) = groovy_home() else { return Vec::new() };
    let lib = home.join("lib");
    let Ok(entries) = std::fs::read_dir(&lib) else { return Vec::new() };
    let cache_base = home.join("bearwisdom-groovy-stdlib-cache");
    let _ = std::fs::create_dir_all(&cache_base);
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        // groovy-X.Y.Z-sources.jar and groovy-all-X.Y.Z-sources.jar
        if !name.starts_with("groovy") || !name.ends_with("-sources.jar") { continue }
        let cache_dir = cache_base.join(name.trim_end_matches(".jar"));
        if !cache_dir.exists() || is_cache_stale(&path, &cache_dir) {
            if let Err(e) = extract_java_sources_jar(&path, &cache_dir) {
                debug!("Failed to extract {}: {e}", path.display());
                continue;
            }
        }
        out.push(ExternalDepRoot {
            module_path: name.trim_end_matches("-sources.jar").to_string(),
            version: String::new(),
            root: cache_dir,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }
    out
}

fn groovy_home() -> Option<PathBuf> {
    for var in ["GROOVY_HOME", "GROOVY_ROOT"] {
        if let Ok(v) = std::env::var(var) {
            let p = PathBuf::from(v);
            if p.is_dir() { return Some(p); }
        }
    }
    None
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<GroovyStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(GroovyStdlibEcosystem)).clone()
}
