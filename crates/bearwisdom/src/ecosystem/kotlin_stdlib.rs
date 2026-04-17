// =============================================================================
// ecosystem/kotlin_stdlib.rs — Kotlin stdlib (stdlib ecosystem)
//
// Probes `$KOTLIN_HOME/lib/kotlin-stdlib-sources.jar` (or a bundled
// equivalent) and extracts Kotlin source into a bearwisdom-owned cache.
// Falls back to a maven-resolved `org.jetbrains.kotlin:kotlin-stdlib`
// sources jar in `~/.m2/repository` when Maven is active.
//
// Unlike Android SDK, Kotlin stdlib is relevant even in pure-Kotlin
// non-Android projects, so activation is just `LanguagePresent("kotlin")`.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{
    extract_java_sources_jar, is_cache_stale, maven_local_repo, ExternalDepRoot,
    ExternalSourceLocator,
};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("kotlin-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "kotlin-stdlib";
const LANGUAGES: &[&str] = &["kotlin"];

pub struct KotlinStdlibEcosystem;

impl Ecosystem for KotlinStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("kotlin")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_kotlin_stdlib_roots()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }
}

impl ExternalSourceLocator for KotlinStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_kotlin_stdlib_roots()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }
}

fn discover_kotlin_stdlib_roots() -> Vec<ExternalDepRoot> {
    // Strategy:
    // 1. $BEARWISDOM_KOTLIN_STDLIB_JAR → explicit path to sources jar.
    // 2. $KOTLIN_HOME/lib/kotlin-stdlib-sources.jar (and kotlin-stdlib-jdk*-sources.jar).
    // 3. Maven-resolved sources jar when Maven is reachable.
    let mut jars: Vec<PathBuf> = Vec::new();

    if let Some(explicit) = std::env::var_os("BEARWISDOM_KOTLIN_STDLIB_JAR") {
        let p = PathBuf::from(explicit);
        if p.is_file() { jars.push(p); }
    }

    if let Some(home) = kotlin_home() {
        let lib = home.join("lib");
        if let Ok(entries) = std::fs::read_dir(&lib) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
                if name.starts_with("kotlin-stdlib")
                    && name.ends_with("-sources.jar")
                {
                    jars.push(path);
                }
            }
        }
    }

    if jars.is_empty() {
        jars.extend(maven_resolved_kotlin_stdlib_jars());
    }

    if jars.is_empty() { return Vec::new() }

    let cache_base = cache_base_for(&jars[0]);
    let _ = std::fs::create_dir_all(&cache_base);

    let mut roots = Vec::new();
    for jar in &jars {
        let stem = jar
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("kotlin-stdlib");
        let cache_dir = cache_base.join(stem);
        if !cache_dir.exists() || is_cache_stale(jar, &cache_dir) {
            if let Err(e) = extract_java_sources_jar(jar, &cache_dir) {
                debug!("Failed to extract {}: {e}", jar.display());
                continue;
            }
        }
        roots.push(ExternalDepRoot {
            module_path: stem.to_string(),
            version: String::new(),
            root: cache_dir,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
        });
    }
    roots
}

fn kotlin_home() -> Option<PathBuf> {
    for var in ["KOTLIN_HOME", "KOTLINC_HOME", "KOTLIN_ROOT"] {
        if let Ok(val) = std::env::var(var) {
            let p = PathBuf::from(val);
            if p.is_dir() { return Some(p) }
        }
    }
    None
}

fn maven_resolved_kotlin_stdlib_jars() -> Vec<PathBuf> {
    let Some(repo) = maven_local_repo() else { return Vec::new() };
    let base = repo.join("org").join("jetbrains").join("kotlin");
    let candidates = ["kotlin-stdlib", "kotlin-stdlib-jdk7", "kotlin-stdlib-jdk8"];
    let mut out = Vec::new();
    for artifact in candidates {
        let art_dir = base.join(artifact);
        if !art_dir.is_dir() { continue }
        let Ok(entries) = std::fs::read_dir(&art_dir) else { continue };
        let mut versions: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.path())
            .collect();
        versions.sort();
        let Some(v_dir) = versions.into_iter().next_back() else { continue };
        let Ok(files) = std::fs::read_dir(&v_dir) else { continue };
        for f in files.flatten() {
            let p = f.path();
            if p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("-sources.jar"))
            {
                out.push(p);
                break;
            }
        }
    }
    out
}

fn cache_base_for(primary_jar: &Path) -> PathBuf {
    primary_jar
        .parent()
        .map(|p| p.join("bearwisdom-kotlin-stdlib-cache"))
        .unwrap_or_else(|| PathBuf::from("bearwisdom-kotlin-stdlib-cache"))
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<KotlinStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(KotlinStdlibEcosystem)).clone()
}
