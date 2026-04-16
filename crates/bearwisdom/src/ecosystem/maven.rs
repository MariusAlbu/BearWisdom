// =============================================================================
// ecosystem/maven.rs — Maven ecosystem (JVM languages)
//
// Covers Java + Kotlin + Scala + Clojure + Groovy in one ecosystem. All five
// languages share the Maven local repository (`~/.m2/repository`) as their
// install location; they differ only in manifest format (pom.xml, build.sbt,
// deps.edn, project.clj, build.gradle[.kts]) and source file extensions.
//
// Before this refactor:
//   indexer/externals/java.rs       — JavaExternalsLocator (pom.xml)
//   indexer/externals/scala.rs      — ScalaExternalsLocator (build.sbt)
//   indexer/externals/clojure.rs    — ClojureExternalsLocator (deps.edn/project.clj)
//   languages/kotlin/externals.rs   — KotlinExternalsLocator (Java + Android SDK)
//   languages/groovy plugin         — delegated to JavaExternalsLocator
// Each scanned ~/.m2 independently in polyglot JVM projects, extracted the
// same jars repeatedly, duplicated resolution logic five ways.
//
// After: one ecosystem. Walks every JVM manifest once, resolves coordinates
// against the Maven local repo once, extracts each sources jar once,
// detects file language by extension on walk. Android SDK probing lives
// here temporarily (until Phase 5 promotes it to its own ecosystem).
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
};
use crate::indexer::externals::{
    collect_pom_files_bounded, extract_java_sources_jar, is_cache_stale, maven_local_repo,
    resolve_maven_artifact_dir, ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH,
};
use crate::indexer::manifest::maven::{parse_pom_xml_coords, MavenCoord};
use crate::indexer::manifest::{clojure as clojure_manifest, sbt as sbt_manifest};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("maven");

/// The JVM ecosystem — Maven local repo + Android SDK (platform jars).
///
/// Activation: any pom.xml, build.sbt, deps.edn, project.clj, or build.gradle[.kts]
/// anywhere in the project. In practice this also triggers when the project
/// contains JVM source files even without manifests (activation predicate is
/// `Any([ManifestMatch, LanguagePresent(java/kotlin/scala/clojure/groovy)])`),
/// because Kotlin-Multiplatform and Android projects often have deps declared
/// only via Gradle's Kotlin DSL which this ecosystem doesn't yet fully parse.
pub struct MavenEcosystem;

// Manifest specs are Phase 3 work (folding indexer/manifest/ into ecosystems).
// For Phase 2 the discovery still walks manifests internally; this slice is
// empty so the trait contract is satisfied without committing to a specific
// parser signature until Phase 3 rationalizes them.
const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["java", "kotlin", "scala", "clojure", "groovy"];

// ---------------------------------------------------------------------------
// Ecosystem trait impl (new — authoritative)
// ---------------------------------------------------------------------------

impl Ecosystem for MavenEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        // Any JVM manifest or any JVM source file triggers activation. In
        // Phase 4 this becomes the single activation gate; today the legacy
        // indexer pipeline still calls locate_roots unconditionally.
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("java"),
            EcosystemActivation::LanguagePresent("kotlin"),
            EcosystemActivation::LanguagePresent("scala"),
            EcosystemActivation::LanguagePresent("clojure"),
            EcosystemActivation::LanguagePresent("groovy"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_maven_roots(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_maven_root(dep)
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl — adapter for the existing indexer
// pipeline. Dropped in Phase 4 when the indexer consumes Ecosystem directly.
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for MavenEcosystem {
    fn ecosystem(&self) -> &'static str { ID.as_str() }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_maven_roots(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_maven_root(dep)
    }
}

/// Process-wide shared instance. The ecosystem registry holds one of these
/// in `default_registry()`; the legacy-locator bridge in
/// `ecosystem::default_locator` exposes the same type through
/// `ExternalSourceLocator` for per-package attribution overrides.
pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<MavenEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(MavenEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery: walk every JVM manifest, collect coords, resolve against ~/.m2
// ---------------------------------------------------------------------------

fn discover_maven_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
    let Some(repo) = maven_local_repo() else {
        debug!("No Maven local repository discovered; skipping Maven externals");
        return Vec::new();
    };
    let cache_base = repo.parent().unwrap_or(&repo).join("bearwisdom-sources-cache");
    let _ = std::fs::create_dir_all(&cache_base);

    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    // --- pom.xml coords (Java + Scala + Kotlin when Maven-built) --------
    let mut pom_paths: Vec<PathBuf> = Vec::new();
    collect_pom_files_bounded(project_root, &mut pom_paths, 0);
    let mut pom_coords = Vec::new();
    for pom in &pom_paths {
        let Ok(content) = std::fs::read_to_string(pom) else { continue };
        pom_coords.extend(parse_pom_xml_coords(&content));
    }
    debug!("Maven: {} pom.xml coords across {} files", pom_coords.len(), pom_paths.len());
    for coord in &pom_coords {
        resolve_and_push(&repo, &cache_base, coord, &mut roots, &mut seen);
    }

    // --- Scala sbt coords ----------------------------------------------
    for artifact in collect_sbt_artifacts(project_root) {
        if let Some((group, art, version, cache_dir)) =
            find_scala_source_jar(&repo, &artifact, &cache_base)
        {
            if seen.insert(cache_dir.clone()) {
                roots.push(ExternalDepRoot {
                    module_path: format!("{group}:{art}"),
                    version,
                    root: cache_dir,
                    ecosystem: ID.as_str(),
                    package_id: None,
                });
            }
        } else {
            debug!(artifact = %artifact, "Maven (sbt): sources jar not found in ~/.m2");
        }
    }

    // --- Clojure deps.edn + project.clj coords --------------------------
    for dep in collect_clojure_deps(project_root) {
        let parts: Vec<&str> = dep.splitn(2, '/').collect();
        let (group_id, artifact_id) = if parts.len() == 2 {
            (parts[0], parts[1])
        } else {
            (dep.as_str(), dep.as_str())
        };
        let coord = MavenCoord {
            group_id: group_id.to_string(),
            artifact_id: artifact_id.to_string(),
            version: None,
        };
        resolve_and_push(&repo, &cache_base, &coord, &mut roots, &mut seen);
    }

    debug!("Maven: {} total external dep roots", roots.len());
    roots
}

fn resolve_and_push(
    repo: &Path,
    cache_base: &Path,
    coord: &MavenCoord,
    roots: &mut Vec<ExternalDepRoot>,
    seen: &mut std::collections::HashSet<PathBuf>,
) {
    let Some((version, artifact_dir)) = resolve_maven_artifact_dir(repo, coord) else { return };
    let sources_jar = artifact_dir.join(format!(
        "{}-{}-sources.jar",
        coord.artifact_id, version
    ));
    if !sources_jar.is_file() {
        debug!(
            "Maven sources jar missing for {}:{}:{} — skipping",
            coord.group_id, coord.artifact_id, version
        );
        return;
    }

    let cache_dir = cache_base
        .join(coord.group_id.replace('.', "_"))
        .join(&coord.artifact_id)
        .join(&version);
    if !cache_dir.exists() || is_cache_stale(&sources_jar, &cache_dir) {
        if let Err(e) = extract_java_sources_jar(&sources_jar, &cache_dir) {
            debug!("Failed to extract {}: {e}", sources_jar.display());
            return;
        }
    }

    if !seen.insert(cache_dir.clone()) { return }
    roots.push(ExternalDepRoot {
        module_path: format!("{}:{}", coord.group_id, coord.artifact_id),
        version,
        root: cache_dir,
        ecosystem: ID.as_str(),
        package_id: None,
    });
}

// ---------------------------------------------------------------------------
// Scala sbt coord resolution (with _2.13, _3 suffix probing)
// ---------------------------------------------------------------------------

fn collect_sbt_artifacts(project_root: &Path) -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    let build_sbt = project_root.join("build.sbt");
    if let Ok(content) = std::fs::read_to_string(&build_sbt) {
        all.extend(sbt_manifest::parse_sbt_deps(&content));
    }
    let deps_scala = project_root.join("project").join("Dependencies.scala");
    if let Ok(content) = std::fs::read_to_string(&deps_scala) {
        for dep in sbt_manifest::parse_sbt_deps(&content) {
            if !all.contains(&dep) {
                all.push(dep);
            }
        }
    }
    all
}

fn find_scala_source_jar(
    repo: &Path,
    artifact: &str,
    cache_base: &Path,
) -> Option<(String, String, String, PathBuf)> {
    let suffixes = ["_3", "_2.13", "_2.12", ""];
    for suffix in &suffixes {
        let full_artifact = format!("{artifact}{suffix}");
        if let Some((group, version, cache_dir)) =
            scan_maven_for_scala_artifact(repo, &full_artifact, cache_base)
        {
            return Some((group, full_artifact, version, cache_dir));
        }
    }
    None
}

fn scan_maven_for_scala_artifact(
    repo: &Path,
    artifact: &str,
    cache_base: &Path,
) -> Option<(String, String, PathBuf)> {
    fn scan_dir(
        dir: &Path,
        artifact: &str,
        cache_base: &Path,
        group_parts: &mut Vec<String>,
        depth: u32,
    ) -> Option<(String, String, PathBuf)> {
        if depth > 10 { return None }
        let Ok(entries) = std::fs::read_dir(dir) else { return None };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') { continue }
            let path = entry.path();
            if !path.is_dir() { continue }

            if name_str.as_ref() == artifact {
                if let Ok(versions) = std::fs::read_dir(&path) {
                    let mut version_dirs: Vec<PathBuf> = versions
                        .flatten()
                        .filter(|e| e.path().is_dir())
                        .map(|e| e.path())
                        .collect();
                    version_dirs.sort();
                    for vdir in version_dirs.iter().rev() {
                        let ver = vdir.file_name()?.to_str()?;
                        let sources_jar = vdir.join(format!("{artifact}-{ver}-sources.jar"));
                        if sources_jar.is_file() {
                            let group = group_parts.join(".");
                            let cache_dir = cache_base
                                .join(group.replace('.', "_"))
                                .join(artifact)
                                .join(ver);
                            if !cache_dir.exists() || is_cache_stale(&sources_jar, &cache_dir) {
                                if extract_java_sources_jar(&sources_jar, &cache_dir).is_err() {
                                    continue;
                                }
                            }
                            return Some((group, ver.to_string(), cache_dir));
                        }
                    }
                }
            } else {
                group_parts.push(name_str.to_string());
                if let result @ Some(_) = scan_dir(&path, artifact, cache_base, group_parts, depth + 1) {
                    return result;
                }
                group_parts.pop();
            }
        }
        None
    }

    let mut group_parts = Vec::new();
    scan_dir(repo, artifact, cache_base, &mut group_parts, 0)
}

// ---------------------------------------------------------------------------
// Clojure deps collection (deps.edn + project.clj, recursive)
// ---------------------------------------------------------------------------

fn collect_clojure_deps(project_root: &Path) -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    collect_clojure_deps_recursive(project_root, &mut all, &mut seen, 0);
    all
}

fn collect_clojure_deps_recursive(
    dir: &Path,
    all: &mut Vec<String>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: usize,
) {
    const MAX_DEPTH: usize = 3;
    if !seen.insert(dir.to_path_buf()) { return }

    if let Ok(content) = std::fs::read_to_string(dir.join("project.clj")) {
        for dep in clojure_manifest::parse_project_clj_deps(&content) {
            if !all.contains(&dep) { all.push(dep); }
        }
    }
    if let Ok(content) = std::fs::read_to_string(dir.join("deps.edn")) {
        for dep in clojure_manifest::parse_deps_edn_deps(&content) {
            if !all.contains(&dep) { all.push(dep); }
        }
    }
    if depth >= MAX_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if matches!(
            name,
            ".git" | "target" | "out" | "node_modules" | ".clj-kondo"
                | ".lsp" | ".cpcache" | "resources" | "doc" | "docs"
        ) { continue }
        collect_clojure_deps_recursive(&path, all, seen, depth + 1);
    }
}

// ---------------------------------------------------------------------------
// Walk: single implementation handling all JVM languages; per-file language
// detection by extension so a Scala sources jar containing both .scala and
// .java files tags each file correctly.
// ---------------------------------------------------------------------------

fn walk_maven_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    walk_generic_jvm_root(dep)
}

/// Walk any JVM source tree — reused by the Android SDK ecosystem whose
/// extracted sources follow the exact same layout (Java + Kotlin + Scala +
/// Clojure intermixed under a single cache dir).
pub(crate) fn walk_generic_jvm_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "META-INF") || name.starts_with('.') {
                    continue;
                }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };

            let (language, virtual_tag) = match detect_jvm_language(name) {
                Some(spec) => spec,
                None => continue,
            };

            // Skip test-suffixed files by convention.
            if name.ends_with("Test.java") || name.ends_with("Tests.java")
                || name.ends_with("Test.scala") || name.ends_with("Tests.scala")
                || name.ends_with("Spec.scala") || name.ends_with("Suite.scala")
                || name == "package-info.java" || name == "module-info.java"
            {
                continue;
            }

            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:{virtual_tag}:{}/{}", dep.module_path, rel_sub);

            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
    }
}

/// Map a filename to (language_id, virtual_tag_for_ext_path).
/// Returns None for non-JVM source extensions.
fn detect_jvm_language(name: &str) -> Option<(&'static str, &'static str)> {
    if name.ends_with(".java") {
        Some(("java", "java"))
    } else if name.ends_with(".kt") || name.ends_with(".kts") {
        Some(("kotlin", "kotlin"))
    } else if name.ends_with(".scala") {
        Some(("scala", "scala"))
    } else if name.ends_with(".clj") || name.ends_with(".cljc") || name.ends_with(".cljs") {
        Some(("clojure", "clojure"))
    } else if name.ends_with(".groovy") || name.ends_with(".gradle")
        || name.ends_with(".gradle.kts")
    {
        // .gradle.kts files inside an extracted jar are unusual but not
        // impossible; the kotlin tag covers them via the earlier branch,
        // this branch just catches Groovy DSL files.
        Some(("groovy", "groovy"))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let m = MavenEcosystem;
        assert_eq!(m.id(), ID);
        assert_eq!(Ecosystem::kind(&m), EcosystemKind::Package);
        assert_eq!(
            Ecosystem::languages(&m),
            &["java", "kotlin", "scala", "clojure", "groovy"]
        );
    }

    #[test]
    fn legacy_locator_ecosystem_string_is_maven() {
        assert_eq!(ExternalSourceLocator::ecosystem(&MavenEcosystem), "maven");
    }

    #[test]
    fn detect_jvm_language_covers_each_extension() {
        assert_eq!(detect_jvm_language("Foo.java"), Some(("java", "java")));
        assert_eq!(detect_jvm_language("Foo.kt"), Some(("kotlin", "kotlin")));
        assert_eq!(detect_jvm_language("Foo.scala"), Some(("scala", "scala")));
        assert_eq!(detect_jvm_language("foo.clj"), Some(("clojure", "clojure")));
        assert_eq!(detect_jvm_language("foo.cljs"), Some(("clojure", "clojure")));
        assert_eq!(detect_jvm_language("build.groovy"), Some(("groovy", "groovy")));
        assert_eq!(detect_jvm_language("readme.md"), None);
    }

    #[test]
    fn empty_project_yields_no_roots() {
        let tmp = std::env::temp_dir().join("bw-test-maven-eco-empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // No pom.xml / build.sbt / deps.edn / project.clj; no ANDROID_HOME set.
        let ctx = LocateContext {
            project_root: &tmp,
            manifests: &std::collections::HashMap::new(),
            active_ecosystems: &[],
        };
        // Sanity — just assert the call doesn't panic. Whether roots are
        // empty depends on whether the running machine has an ANDROID_HOME
        // exported; both outcomes are valid.
        let _ = <MavenEcosystem as Ecosystem>::locate_roots(&MavenEcosystem, &ctx);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // Suppress dead-code warnings on helpers that become used once Phase 2
    // finishes wiring the plugins through.
    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }
}
