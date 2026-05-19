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

    fn supports_reachability(&self) -> bool { true }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(
        &self,
        dep_roots: &[crate::ecosystem::externals::ExternalDepRoot],
    ) -> crate::ecosystem::symbol_index::SymbolLocationIndex {
        super::maven::build_maven_symbol_index(dep_roots)
    }

    /// Pre-pull every `java.base/java/lang/*.java` file at Stage 2.
    /// `java.lang` is the only Java package implicitly imported by every
    /// compilation unit — `String`, `Integer`, `Object`, `Exception`,
    /// `Throwable`, `Iterable`, `Boolean`, `Number`, `Class`,
    /// `RuntimeException`, etc. are referenced by bare name across every
    /// Java project. Pure demand-driven walking never pulls them because
    /// project refs carry no `java.lang.` FQN to trigger the resolve.
    /// On java-spring-petclinic this absence accounted for 95 of the 127
    /// missing internal_edges (all bare `String` type_refs).
    ///
    /// java.lang fits well inside the demand-pre-pull contract: it's a
    /// bounded set (~145 files), every Java project needs the same set,
    /// and the walk runs once per indexing pass rather than per ref.
    fn demand_pre_pull(
        &self,
        dep_roots: &[crate::ecosystem::externals::ExternalDepRoot],
    ) -> Vec<WalkedFile> {
        let mut out = Vec::new();
        for dep in dep_roots {
            collect_java_lang_files(dep, &mut out);
        }
        out
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
        requested_imports: Vec::new(),
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

/// Collect every `.java` file under `dep.root/java.base/java/lang/`. The
/// JDK src.zip extracts into a `<module>/<package>/<file>.java` layout;
/// java.lang lives under `java.base/java/lang/`. Anything outside that
/// directory stays demand-driven via the regular walker.
fn collect_java_lang_files(
    dep: &crate::ecosystem::externals::ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
) {
    let java_lang = dep.root.join("java.base").join("java").join("lang");
    let Ok(entries) = std::fs::read_dir(&java_lang) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !name.ends_with(".java") {
            continue;
        }
        // Skip module-info / package-info — these contain compile-time
        // metadata, not symbols the resolver needs.
        if name == "module-info.java" || name == "package-info.java" {
            continue;
        }
        let rel_sub = match path.strip_prefix(&dep.root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let virtual_path = format!("ext:java:{}/{}", dep.module_path, rel_sub);
        out.push(WalkedFile {
            relative_path: virtual_path,
            absolute_path: path,
            language: "java",
        });
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<JdkSrcEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(JdkSrcEcosystem)).clone()
}
