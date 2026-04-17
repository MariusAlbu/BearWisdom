// =============================================================================
// ecosystem/jdk_src.rs — JDK standard library sources (stdlib ecosystem)
//
// Probes `$JAVA_HOME/lib/src.zip` (JDK 9+) or `$JAVA_HOME/src.zip`
// (legacy) and extracts its .java sources into a bearwisdom-owned cache.
// Reuses the Maven sources-jar extraction helper — the zip layout is
// compatible.
//
// Serves Java + Kotlin + Scala + Clojure because all four languages
// resolve JDK types (String, List, Map, etc.) in plain Java form.
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

pub const ID: EcosystemId = EcosystemId::new("jdk-src");
const LEGACY_ECOSYSTEM_TAG: &str = "jdk-src";
const LANGUAGES: &[&str] = &["java", "kotlin", "scala", "clojure"];

pub struct JdkSrcEcosystem;

impl Ecosystem for JdkSrcEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("java"),
            EcosystemActivation::LanguagePresent("kotlin"),
            EcosystemActivation::LanguagePresent("scala"),
            EcosystemActivation::LanguagePresent("clojure"),
            EcosystemActivation::LanguagePresent("groovy"),
        ])
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_jdk_src_roots()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }
}

impl ExternalSourceLocator for JdkSrcEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_jdk_src_roots()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }
}

fn discover_jdk_src_roots() -> Vec<ExternalDepRoot> {
    let Some(src_zip) = probe_src_zip() else {
        debug!("jdk-src: no src.zip found");
        return Vec::new();
    };
    let Some(cache_base) = jdk_src_cache_dir() else {
        debug!("jdk-src: no writable cache directory");
        return Vec::new();
    };
    let cache_dir = cache_base.join("jdk-src");
    if !cache_dir.exists() || is_cache_stale(&src_zip, &cache_dir) {
        if let Err(e) = extract_java_sources_jar(&src_zip, &cache_dir) {
            debug!("Failed to extract {}: {e}", src_zip.display());
            return Vec::new();
        }
    }
    debug!("jdk-src: extracted to {}", cache_dir.display());
    vec![ExternalDepRoot {
        module_path: "jdk".to_string(),
        version: String::new(),
        root: cache_dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
    }]
}

fn jdk_src_cache_dir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_JDK_SRC_CACHE") {
        let p = PathBuf::from(explicit);
        std::fs::create_dir_all(&p).ok()?;
        return Some(p);
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let p = PathBuf::from(local).join("bearwisdom").join("jdk-src-cache");
        if std::fs::create_dir_all(&p).is_ok() { return Some(p); }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home).join(".cache").join("bearwisdom").join("jdk-src-cache");
        if std::fs::create_dir_all(&p).is_ok() { return Some(p); }
    }
    None
}

fn probe_src_zip() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_JDK_SRC_ZIP") {
        let p = PathBuf::from(explicit);
        if p.is_file() { return Some(p); }
    }
    let home = std::env::var_os("JAVA_HOME").map(PathBuf::from)?;
    // JDK 9+: $JAVA_HOME/lib/src.zip
    let modern = home.join("lib").join("src.zip");
    if modern.is_file() { return Some(modern); }
    // Legacy JDK 8: $JAVA_HOME/src.zip
    let legacy = home.join("src.zip");
    if legacy.is_file() { return Some(legacy); }
    None
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<JdkSrcEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(JdkSrcEcosystem)).clone()
}
