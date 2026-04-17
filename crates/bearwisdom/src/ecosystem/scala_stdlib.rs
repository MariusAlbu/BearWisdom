// =============================================================================
// ecosystem/scala_stdlib.rs — Scala stdlib (stdlib ecosystem)
//
// Probes a maven-resolved `org.scala-lang:scala-library` sources jar in
// the local ~/.m2 repo (Scala's stdlib is shipped as a normal Maven
// artifact). Activation: LanguagePresent("scala"). TransitiveOn(maven)
// keeps the probe cheap.
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

pub const ID: EcosystemId = EcosystemId::new("scala-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "scala-stdlib";
const LANGUAGES: &[&str] = &["scala"];

pub struct ScalaStdlibEcosystem;

impl Ecosystem for ScalaStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("scala")
    }
    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }
}

impl ExternalSourceLocator for ScalaStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> { discover() }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }
}

fn discover() -> Vec<ExternalDepRoot> {
    let Some(repo) = maven_local_repo() else { return Vec::new() };
    // Scala 2.x: org/scala-lang/scala-library/X.Y.Z/scala-library-X.Y.Z-sources.jar
    // Scala 3.x: org/scala-lang/scala3-library_3/X.Y.Z/scala3-library_3-X.Y.Z-sources.jar
    let candidates = [
        ("org/scala-lang", "scala-library"),
        ("org/scala-lang", "scala3-library_3"),
    ];
    let cache_base = repo.parent().unwrap_or(&repo).join("bearwisdom-scala-stdlib-cache");
    let _ = std::fs::create_dir_all(&cache_base);
    let mut out = Vec::new();
    for (group, artifact) in candidates {
        let mut group_path = repo.clone();
        for seg in group.split('/') { group_path.push(seg); }
        group_path.push(artifact);
        if !group_path.is_dir() { continue }
        let Ok(versions) = std::fs::read_dir(&group_path) else { continue };
        let mut vs: Vec<PathBuf> = versions
            .flatten().filter(|e| e.path().is_dir()).map(|e| e.path()).collect();
        vs.sort();
        let Some(latest) = vs.into_iter().next_back() else { continue };
        let Ok(files) = std::fs::read_dir(&latest) else { continue };
        for f in files.flatten() {
            let p = f.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with("-sources.jar") { continue }
            let cache_dir = cache_base.join(name.trim_end_matches(".jar"));
            if !cache_dir.exists() || is_cache_stale(&p, &cache_dir) {
                if let Err(e) = extract_java_sources_jar(&p, &cache_dir) {
                    debug!("Failed to extract {}: {e}", p.display());
                    continue;
                }
            }
            out.push(ExternalDepRoot {
                module_path: format!("{group}:{artifact}"),
                version: latest.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string(),
                root: cache_dir,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
            break;
        }
    }
    out
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<ScalaStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(ScalaStdlibEcosystem)).clone()
}
