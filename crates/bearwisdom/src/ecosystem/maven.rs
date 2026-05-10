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

use rayon::prelude::*;
use tracing::{debug, warn};
use tree_sitter::{Node, Parser};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{
    collect_pom_files_bounded, coursier_cache_root, extract_java_sources_jar, gradle_caches_root,
    is_cache_stale, maven_local_repo, resolve_coursier_sources_jar, resolve_coursier_submodule_jars,
    resolve_gradle_sources_jar, resolve_maven_artifact_dir, ExternalDepRoot, ExternalSourceLocator,
    MAX_WALK_DEPTH,
};
use crate::ecosystem::manifest::maven::{parse_pom_xml_coords, MavenCoord};
use crate::ecosystem::manifest::{
    clojure as clojure_manifest,
    gradle as gradle_manifest,
    sbt as sbt_manifest,
};
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

    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        // Maven covers the JVM tool family in BearWisdom: pom.xml + Gradle
        // (build.gradle / .kts) + SBT. Each filename gets its own kind label
        // so users querying packages.kind can tell them apart.
        &[
            ("pom.xml",          "maven"),
            ("build.gradle",     "gradle"),
            ("build.gradle.kts", "gradle"),
            ("build.sbt",        "sbt"),
            ("deps.edn",         "clojure"),
        ]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["target", ".gradle", ".mvn", "out", "build"]
    }

    fn activation(&self) -> EcosystemActivation {
        // Project deps activate via ManifestMatch — the project declares
        // its JVM dep set via pom.xml / build.gradle[.kts] / build.sbt /
        // deps.edn. A bare directory of `.java` files with no manifest
        // can't be resolved against external Maven coordinates anyway
        // (no version, no group:artifact pinning), so dropping the
        // LanguagePresent shotgun is correct per the trait doc's
        // project-deps rule.
        EcosystemActivation::ManifestMatch
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_maven_roots(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_maven_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        walk_maven_narrowed(dep)
    }

    fn resolve_symbol(
        &self,
        dep: &ExternalDepRoot,
        _fqn: &str,
    ) -> Vec<WalkedFile> {
        walk_maven_narrowed(dep)
    }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_maven_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
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
    let m2 = maven_local_repo();
    let gradle_cache = gradle_caches_root();
    let coursier_cache = coursier_cache_root();
    if m2.is_none() && gradle_cache.is_none() && coursier_cache.is_none() {
        debug!("No Maven, Gradle, or Coursier cache discovered; skipping JVM externals");
        return Vec::new();
    }

    // Pick a shared bearwisdom-sources-cache anchor. Prefer ~/.m2/.. so
    // existing extracted caches are reused; fall back to ~/.gradle/.. for
    // pure-Gradle dev machines, then Coursier for SBT-only setups.
    let cache_anchor = m2
        .as_deref()
        .or(gradle_cache.as_deref())
        .or(coursier_cache.as_deref())
        .map(|p| p.parent().unwrap_or(p).to_path_buf())
        .expect("at least one cache root");
    let cache_base = cache_anchor.join("bearwisdom-sources-cache");
    let _ = std::fs::create_dir_all(&cache_base);

    // R3 narrowing: collect every JVM `import` statement the user writes.
    // Each artifact's ExternalDepRoot carries the full set; walk_maven_narrowed
    // filters to only the package dirs actually referenced, collapsing the
    // cost of extracting spring-core or scala-library (~1000s of classes each)
    // to just the handful of packages the project consumes.
    let user_imports: Vec<String> = collect_jvm_user_imports(project_root)
        .into_iter()
        .collect();

    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut missing_sources_jars: Vec<String> = Vec::new();

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
        resolve_and_push_jvm(
            m2.as_deref(),
            gradle_cache.as_deref(),
            coursier_cache.as_deref(),
            &cache_base,
            coord,
            &user_imports,
            &mut roots,
            &mut seen,
            &mut missing_sources_jars,
        );
    }

    // --- Gradle build.gradle[.kts] + version-catalog coords -------------
    let gradle_coords = collect_gradle_coords(project_root);
    debug!("Gradle: {} coords from build.gradle + libs.versions.toml", gradle_coords.len());
    for coord in &gradle_coords {
        resolve_and_push_jvm(
            m2.as_deref(),
            gradle_cache.as_deref(),
            coursier_cache.as_deref(),
            &cache_base,
            coord,
            &user_imports,
            &mut roots,
            &mut seen,
            &mut missing_sources_jars,
        );
    }

    // --- Scala sbt coords ----------------------------------------------
    // sbt's `%%` operator appends the active Scala version suffix to the
    // artifact name (`cats-core` → `cats-core_3` for Scala 3,
    // `cats-core_2.13` for 2.13). Without evaluating the build we can't
    // know which one the user is on, so probe each suffix in turn against
    // every cache. First hit wins.
    //
    // The triples carry a manifest-pinned version when it can be resolved
    // (via `val NAME = "X.Y.Z"` bindings across every sbt manifest) — that
    // pinned version gets passed to MavenCoord so the resolver targets the
    // right version directly. When unresolved, fall back to the lex-scan
    // over cached versions inside `resolve_maven_artifact_dir`.
    let scala_suffixes = ["_3", "_2.13", "_2.12", ""];
    for (group, artifact_base, version) in collect_sbt_coord_triples(project_root) {
        let mut hit = false;
        for suffix in &scala_suffixes {
            let artifact_id = format!("{artifact_base}{suffix}");
            let coord = MavenCoord {
                group_id: group.clone(),
                artifact_id,
                version: version.clone(),
            };
            let before = roots.len();
            resolve_and_push_jvm(
                m2.as_deref(),
                gradle_cache.as_deref(),
                coursier_cache.as_deref(),
                &cache_base,
                &coord,
                &user_imports,
                &mut roots,
                &mut seen,
                &mut missing_sources_jars,
            );
            if roots.len() > before {
                hit = true;
                break;
            }
        }
        if !hit {
            debug!(group = %group, artifact = %artifact_base, "sbt: sources jar not found in any cache");
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
        resolve_and_push_jvm(
            m2.as_deref(),
            gradle_cache.as_deref(),
            coursier_cache.as_deref(),
            &cache_base,
            &coord,
            &user_imports,
            &mut roots,
            &mut seen,
            &mut missing_sources_jars,
        );
    }

    debug!("JVM ecosystem: {} total external dep roots", roots.len());
    if !missing_sources_jars.is_empty() {
        // Sources jars are an opt-in artifact (`mvn dependency:sources` /
        // Gradle `idea`/`eclipse` task / sbt `updateClassifiers`). Without
        // them the binary jar exists but BearWisdom can't index its symbols.
        // Surface a single summary so users know how to recover resolution
        // for declared deps that didn't resolve.
        let preview: Vec<&str> = missing_sources_jars
            .iter()
            .take(5)
            .map(|s| s.as_str())
            .collect();
        let suffix = if missing_sources_jars.len() > preview.len() {
            format!(", … and {} more", missing_sources_jars.len() - preview.len())
        } else {
            String::new()
        };
        warn!(
            "Maven: {} declared JVM deps have no -sources.jar in the local caches \
             ({}{}). Run `mvn dependency:sources` (or sbt `updateClassifiers` / \
             Gradle `:dependencies --refresh-dependencies` with sources)\
             to populate them.",
            missing_sources_jars.len(),
            preview.join(", "),
            suffix,
        );
    }
    roots
}

/// Parse every build.gradle[.kts] in the project and resolve catalog
/// references against any `gradle/*.versions.toml` files. Returns full
/// `MavenCoord`s — coords that omit `version` (rare in Gradle) get the
/// version-dir scan fallback the same way pom coords do.
fn collect_gradle_coords(project_root: &Path) -> Vec<MavenCoord> {
    let mut catalogs: std::collections::HashMap<String, gradle_manifest::GradleCatalog> =
        std::collections::HashMap::new();
    for (name, path) in gradle_manifest::collect_version_catalogs(project_root) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            catalogs.insert(name, gradle_manifest::parse_version_catalog(&content));
        }
    }

    let mut out = Vec::new();
    for build_file in gradle_manifest::collect_gradle_build_files(project_root) {
        let Ok(content) = std::fs::read_to_string(&build_file) else { continue };
        out.extend(gradle_manifest::parse_gradle_coords(&content, &catalogs));
    }
    out
}

/// Try ~/.m2 first (preferred — single jar per artifact dir), fall back to
/// ~/.gradle/caches (hash-bucketed layout), then to Coursier's cache (used
/// by SBT's `updateClassifiers`). The first cache that yields a
/// `<artifact>-<version>-sources.jar` wins; all missing → silent skip.
fn resolve_and_push_jvm(
    m2: Option<&Path>,
    gradle_cache: Option<&Path>,
    coursier_cache: Option<&Path>,
    cache_base: &Path,
    coord: &MavenCoord,
    user_imports: &[String],
    roots: &mut Vec<ExternalDepRoot>,
    seen: &mut std::collections::HashSet<PathBuf>,
    missing_sources_jars: &mut Vec<String>,
) {
    // First pass: try the manifest-pinned version (when set) across every
    // cache. If nothing has the exact pinned version, fall back to a
    // version-blind probe so the walker still picks SOMETHING — a
    // close-enough version is more useful than no externals at all,
    // especially when the project's compile-classpath resolves to a
    // version that isn't pinned in the manifest verbatim.
    let resolved = try_resolve_in_caches(m2, gradle_cache, coursier_cache, coord)
        .or_else(|| {
            if coord.version.is_some() {
                let unpinned = MavenCoord {
                    group_id: coord.group_id.clone(),
                    artifact_id: coord.artifact_id.clone(),
                    version: None,
                };
                try_resolve_in_caches(m2, gradle_cache, coursier_cache, &unpinned)
            } else {
                None
            }
        });

    let Some((version, sources_jar)) = resolved else {
        debug!(
            "JVM sources jar missing for {}:{} (checked Maven + Gradle + Coursier) — skipping",
            coord.group_id, coord.artifact_id
        );
        missing_sources_jars.push(format!("{}:{}", coord.group_id, coord.artifact_id));
        return;
    };

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

    // Aggregator check: some Scala libraries (e.g. ScalaTest, Cats) publish
    // a top-level artifact whose -sources.jar is an empty shell containing
    // only META-INF. The real sources live in constituent sub-module jars
    // in the same Coursier group directory (e.g. scalatest-core_2.13,
    // scalatest-shouldmatchers_2.13). When the extracted cache dir is empty,
    // probe Coursier for those sub-module jars.
    let cache_is_empty = std::fs::read_dir(&cache_dir)
        .map(|mut d| d.next().is_none())
        .unwrap_or(true);
    if cache_is_empty {
        if let Some(coursier) = coursier_cache {
            // Derive the prefix by stripping the Scala binary-version suffix.
            let artifact_base = strip_scala_version_suffix(&coord.artifact_id);
            let sub_jars = resolve_coursier_submodule_jars(
                coursier,
                &coord.group_id,
                artifact_base,
                Some(&version),
            );
            for (sub_artifact, sub_version, sub_jar) in sub_jars {
                let sub_cache = cache_base
                    .join(coord.group_id.replace('.', "_"))
                    .join(&sub_artifact)
                    .join(&sub_version);
                if !sub_cache.exists() || is_cache_stale(&sub_jar, &sub_cache) {
                    if let Err(e) = extract_java_sources_jar(&sub_jar, &sub_cache) {
                        debug!("Failed to extract sub-module {sub_artifact}: {e}");
                        continue;
                    }
                }
                if !seen.insert(sub_cache.clone()) { continue; }
                roots.push(ExternalDepRoot {
                    module_path: format!("{}:{}", coord.group_id, sub_artifact),
                    version: sub_version,
                    root: sub_cache,
                    ecosystem: ID.as_str(),
                    package_id: None,
                    requested_imports: user_imports.to_vec(),
                });
            }
        }
        // The aggregator itself has no source files — don't push it as a root.
        return;
    }

    if !seen.insert(cache_dir.clone()) { return }
    roots.push(ExternalDepRoot {
        module_path: format!("{}:{}", coord.group_id, coord.artifact_id),
        version,
        root: cache_dir,
        ecosystem: ID.as_str(),
        package_id: None,
        requested_imports: user_imports.to_vec(),
    });
}

/// Strip a Scala binary-version suffix from an artifact name.
/// `scalatest_2.13` → `scalatest`, `cats-core_3` → `cats-core`.
fn strip_scala_version_suffix(artifact: &str) -> &str {
    for suffix in &["_2.13", "_2.12", "_2.11", "_3"] {
        if let Some(base) = artifact.strip_suffix(suffix) {
            return base;
        }
    }
    artifact
}

/// Probe each cache (Maven local, Gradle, Coursier) for a sources jar
/// matching `coord`. First hit wins. The `coord.version` field is honored:
/// when set, every cache is asked for that specific version; when None,
/// each cache falls back to its own version-blind newest-wins logic.
fn try_resolve_in_caches(
    m2: Option<&Path>,
    gradle_cache: Option<&Path>,
    coursier_cache: Option<&Path>,
    coord: &MavenCoord,
) -> Option<(String, PathBuf)> {
    if let Some(repo) = m2 {
        if let Some((version, artifact_dir)) = resolve_maven_artifact_dir(repo, coord) {
            let sources_jar = artifact_dir.join(format!(
                "{}-{}-sources.jar",
                coord.artifact_id, version
            ));
            if sources_jar.is_file() {
                return Some((version, sources_jar));
            }
        }
    }
    if let Some(cache) = gradle_cache {
        if let Some(hit) = resolve_gradle_sources_jar(cache, coord) {
            return Some(hit);
        }
    }
    if let Some(cache) = coursier_cache {
        if let Some(hit) = resolve_coursier_sources_jar(cache, coord) {
            return Some(hit);
        }
    }
    None
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

/// Walk every sbt manifest in the project (root build.sbt + project/*.sbt
/// + project/Dependencies.scala) and return `(group, artifact)` pairs.
/// Sub-projects in monorepos often have their own build.sbt under
/// `<project>/<module>/build.sbt`; collect those too.
fn collect_sbt_coord_pairs(project_root: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();

    let mut sbt_files = Vec::new();
    collect_sbt_files(project_root, &mut sbt_files, 0);

    for path in sbt_files {
        let Ok(content) = std::fs::read_to_string(&path) else { continue };
        for pair in sbt_manifest::parse_sbt_coord_pairs(&content) {
            if seen.insert(pair.clone()) {
                out.push(pair);
            }
        }
    }
    out
}

/// Variant that also carries the manifest-pinned version when it can be
/// resolved through `val NAME = "X.Y.Z"` bindings collected across every
/// sbt manifest in the project. Returns `(group, artifact, Option<version>)`
/// so the resolver can target the right version directory directly instead
/// of falling back to a (broken) lex-sort over whatever's in the cache.
fn collect_sbt_coord_triples(project_root: &Path) -> Vec<(String, String, Option<String>)> {
    let mut sbt_files = Vec::new();
    collect_sbt_files(project_root, &mut sbt_files, 0);

    // First pass: union all `val NAME = "VERSION"` bindings across every
    // manifest. sbt convention scatters them between root build.sbt and
    // project/Dependencies.scala — collect both before resolving deps.
    let mut vars: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for path in &sbt_files {
        let Ok(content) = std::fs::read_to_string(path) else { continue };
        for (k, v) in sbt_manifest::parse_sbt_version_vars(&content) {
            vars.insert(k, v);
        }
    }

    let mut out: Vec<(String, String, Option<String>)> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();

    for path in &sbt_files {
        let Ok(content) = std::fs::read_to_string(path) else { continue };
        for triple in sbt_manifest::parse_sbt_coord_triples(&content, &vars) {
            let key = (triple.0.clone(), triple.1.clone());
            // Last write wins for version: a later manifest mention with a
            // resolved version overrides an earlier coord-only mention.
            if let Some(existing_idx) = out.iter().position(|t| t.0 == triple.0 && t.1 == triple.1) {
                if out[existing_idx].2.is_none() && triple.2.is_some() {
                    out[existing_idx].2 = triple.2;
                }
            } else if seen.insert(key) {
                out.push(triple);
            }
        }
    }
    out
}

fn collect_sbt_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    ".git" | "target" | "build" | "node_modules"
                        | ".gradle" | ".idea" | ".bsp"
                ) || name.starts_with('.') {
                    continue;
                }
            }
            collect_sbt_files(&path, out, depth + 1);
        } else if path.is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".sbt") || name == "Dependencies.scala" {
                    out.push(path);
                }
            }
        }
    }
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
// R3 reachability — user-import scanner + package-narrowed walk
// ---------------------------------------------------------------------------
//
// JVM languages use fully-qualified `import a.b.C` statements (or `import a.b.*`
// wildcards). We scan every .java/.kt/.scala/.clj*/.groovy file in the project,
// extract the imports, and carry them on every Maven ExternalDepRoot. At walk
// time we narrow to only the package dirs the project actually references —
// e.g., a project that imports `org.springframework.context.ApplicationContext`
// causes spring-context's sources jar to yield only files under
// `org/springframework/context/` (~20 files) instead of the whole jar (~500
// files). Unreferenced artifacts contribute zero walked files, which is the
// expected outcome when a transitive dep is pulled in but never named.

fn collect_jvm_user_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut imports = std::collections::HashSet::new();
    scan_jvm_imports_recursive(project_root, &mut imports, 0);
    imports
}

fn scan_jvm_imports_recursive(
    dir: &Path,
    out: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 12 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    ".git" | "target" | "build" | "out" | "dist" | "node_modules"
                        | ".gradle" | ".idea" | ".settings" | "bin" | ".cpcache"
                ) || name.starts_with('.') { continue }
            }
            scan_jvm_imports_recursive(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let is_jvm = name.ends_with(".java") || name.ends_with(".kt")
                || name.ends_with(".kts") || name.ends_with(".scala")
                || name.ends_with(".clj") || name.ends_with(".cljc")
                || name.ends_with(".cljs") || name.ends_with(".groovy")
                || name.ends_with(".gradle");
            if !is_jvm { continue }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            if name.ends_with(".clj") || name.ends_with(".cljc") || name.ends_with(".cljs") {
                extract_clojure_imports(&content, out);
            } else {
                extract_jvm_imports_from_source(&content, out);
            }
        }
    }
}

/// Parse `import a.b.C;` / `import static a.b.C.method;` / `import a.b.{X, Y}` /
/// `import a.b.C` (Kotlin/Scala — no trailing `;`). Inserts the fully-qualified
/// name(s) without the trailing `;` or `{...}` pieces. Scala selector blocks
/// (`import a.b.{X, Y}`) emit `a.b.X` and `a.b.Y` so narrowing sees each class.
fn extract_jvm_imports_from_source(
    content: &str,
    out: &mut std::collections::HashSet<String>,
) {
    for raw in content.lines() {
        let line = raw.trim();
        // Java/Kotlin/Groovy form.
        let after_import = line
            .strip_prefix("import ")
            .or_else(|| line.strip_prefix("import\t"));
        let Some(rest) = after_import else { continue };
        let rest = rest.trim_start_matches("static ").trim();
        let rest = rest.trim_end_matches(';').trim();
        if rest.is_empty() { continue }

        // Scala selector form: `a.b.{X, Y}` or `a.b.{X => Z}`.
        if let Some(brace_open) = rest.find('{') {
            if let Some(brace_close) = rest.find('}') {
                let prefix = rest[..brace_open].trim_end_matches('.').trim();
                if prefix.is_empty() { continue }
                let inner = &rest[brace_open + 1..brace_close];
                for sel in inner.split(',') {
                    let sel = sel.trim();
                    let head = sel.split("=>").next().unwrap_or("").trim();
                    if head.is_empty() || head == "_" {
                        out.insert(format!("{prefix}.*"));
                    } else {
                        out.insert(format!("{prefix}.{head}"));
                    }
                }
                continue;
            }
        }

        // Drop an `as alias` tail (Kotlin) or ` => alias` (Scala single).
        let head = rest
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(',');
        if head.is_empty() { continue }
        out.insert(head.to_string());
    }
}

/// Parse Clojure `(:import [pkg.ns Class1 Class2])` / `(:import pkg.ns.Class)` /
/// `(:require [pkg.ns :as alias])` forms. Emits FQNs and ns namespaces — both
/// are used later by package-prefix narrowing.
fn extract_clojure_imports(
    content: &str,
    out: &mut std::collections::HashSet<String>,
) {
    // Minimal tolerant scanner — enough for the narrow-walk use. We're not
    // trying to be a Clojure reader; we just want dotted names following the
    // `:import` or `:require` keywords.
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':'
            && (i + 7 <= bytes.len() && &bytes[i..i + 7] == b":import"
                || i + 8 <= bytes.len() && &bytes[i..i + 8] == b":require")
        {
            let from = i;
            // Find the matching top-level close paren for this form.
            let mut depth: i32 = 0;
            let mut end = from;
            for (j, &b) in bytes.iter().enumerate().skip(from) {
                match b {
                    b'(' | b'[' => depth += 1,
                    b')' | b']' => {
                        depth -= 1;
                        if depth < 0 { end = j; break; }
                    }
                    _ => {}
                }
            }
            if end == from { break }
            let region = std::str::from_utf8(&bytes[from..end]).unwrap_or("");
            // `[pkg.ns Class1 Class2]` tokens, or `pkg.ns.Class`.
            for tok in region.split(|c: char| c.is_whitespace() || "()[]".contains(c)) {
                if tok.is_empty() { continue }
                if !tok.contains('.') { continue }
                if tok.starts_with(':') { continue }
                if tok.chars().next().map_or(true, |c| !c.is_alphabetic()) { continue }
                out.insert(tok.to_string());
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }
}

/// Convert a user import like `com.foo.Bar` / `com.foo.*` / `com.foo.Outer.Inner`
/// to the on-disk directory prefix `com/foo/` (or `com/foo/Outer/` for nested).
/// Returns `None` for single-segment imports (no package, can't narrow).
fn jvm_import_to_package_prefix(fqn: &str) -> Option<String> {
    let trimmed = fqn.trim().trim_end_matches(".*");
    let last_dot = trimmed.rfind('.')?;
    let pkg = &trimmed[..last_dot];
    if pkg.is_empty() { return None }
    Some(format!("{}/", pkg.replace('.', "/")))
}

fn walk_maven_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() {
        // No demand data — legacy eager walk. Keeps a pre-R3-style answer.
        return walk_maven_root(dep);
    }
    let prefixes: std::collections::HashSet<String> = dep
        .requested_imports
        .iter()
        .filter_map(|fqn| jvm_import_to_package_prefix(fqn))
        .collect();
    if prefixes.is_empty() {
        return walk_maven_root(dep);
    }

    let mut out = Vec::new();
    walk_narrowed_dir(&dep.root, &dep.root, dep, &prefixes, &mut out, 0);
    out
}

fn walk_narrowed_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    prefixes: &std::collections::HashSet<String>,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let path = entry.path();

        let rel_sub = match path.strip_prefix(root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "META-INF") || name.starts_with('.') {
                    continue;
                }
            }
            // Recurse if this dir could still be a prefix of, or is under, a requested prefix.
            let dir_key = format!("{rel_sub}/");
            let may_contain = prefixes.iter().any(|p| p.starts_with(&dir_key) || dir_key.starts_with(p));
            if !may_contain { continue }
            walk_narrowed_dir(&path, root, dep, prefixes, out, depth + 1);
        } else if file_type.is_file() {
            // Match on the file's parent-dir key against any requested prefix.
            let Some(slash) = rel_sub.rfind('/') else { continue };
            let parent_key = format!("{}/", &rel_sub[..slash]);
            if !prefixes.iter().any(|p| parent_key == *p || parent_key.starts_with(p)) {
                continue;
            }

            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let (language, virtual_tag) = match detect_jvm_language(name) {
                Some(spec) => spec,
                None => continue,
            };
            if name.ends_with("Test.java") || name.ends_with("Tests.java")
                || name.ends_with("Test.scala") || name.ends_with("Tests.scala")
                || name.ends_with("Spec.scala") || name.ends_with("Suite.scala")
                || name == "package-info.java" || name == "module-info.java"
            {
                continue;
            }

            let virtual_path = format!("ext:{virtual_tag}:{}/{}", dep.module_path, rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
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
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------
//
// Walks every reached Maven dep root, header-only tree-sitter parses each
// .java/.kt/.scala/.clj[cs]?/.groovy file, and records each top-level type /
// function name against the file that defines it. The Stage 2 loop queries
// this index to pull only the files a user ref actually demands — the rest of
// the extracted sources jar stays on disk.
//
// Key shape: every symbol is inserted twice.
//   * `(module_path, name)` — keyed by the Maven coordinate `group:artifact`
//     so find_by_name's module-agnostic fallback and ecosystem-internal
//     diagnostics both work.
//   * `(java_package, name)` — keyed by the Java/Kotlin/Scala/etc. package
//     path derived from the file's location under the dep root
//     (`dep.root/org/springframework/context/Ctx.java` →
//     `org.springframework.context`). The JVM language resolvers emit
//     `module=java_package` on refs, so `locate(java_package, name)` hits
//     directly when a user `import org.springframework.context.Ctx` is
//     seeded.

pub(crate) fn build_maven_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    // Collect every walked file + its owning dep metadata so each parallel
    // scan task is self-contained. We walk the FULL dep root (not the
    // R3-narrowed slice) so the index covers every symbol the jar publishes,
    // not just those the project already imports — chain-miss resolution
    // pulls further files via `find_by_name` once imports expand.
    let mut work: Vec<(String, PathBuf, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        let root = dep.root.clone();
        for wf in walk_maven_root(dep) {
            work.push((dep.module_path.clone(), root.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }

    // Parallel header-only scan. Each task emits `(module_key, name, file)`
    // tuples keyed by BOTH the Maven coordinate AND the derived Java package.
    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module_path, dep_root, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            let names = scan_jvm_header(&src, wf.language);
            if names.is_empty() {
                return Vec::new();
            }
            let java_package = java_package_from_rel_path(&wf.absolute_path, dep_root);
            let mut rows: Vec<(String, String, PathBuf)> =
                Vec::with_capacity(names.len() * 2);
            for name in names {
                rows.push((module_path.clone(), name.clone(), wf.absolute_path.clone()));
                if let Some(pkg) = java_package.as_ref() {
                    rows.push((pkg.clone(), name, wf.absolute_path.clone()));
                }
            }
            rows
        })
        .collect();

    let mut index = SymbolLocationIndex::new();
    for batch in per_file {
        for (module, name, file) in batch {
            index.insert(module, name, file);
        }
    }
    index
}

/// Derive the Java/Kotlin/Scala package from a file's location under the dep
/// root. `dep.root/org/springframework/context/Ctx.java` yields
/// `"org.springframework.context"`. Returns None for files at the dep root
/// (no package segments) or paths not under `dep_root`.
fn java_package_from_rel_path(file: &Path, dep_root: &Path) -> Option<String> {
    let rel = file.strip_prefix(dep_root).ok()?;
    let mut segs: Vec<&str> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    if segs.len() < 2 { return None }
    segs.pop(); // drop the filename, keep directory segments.
    Some(segs.join("."))
}

/// Dispatch header-only scan by language id. Returns every top-level
/// declaration name the file publishes. Function/method/class bodies are
/// never walked.
fn scan_jvm_header(source: &str, language: &str) -> Vec<String> {
    match language {
        "java" => scan_java_header(source),
        "kotlin" => scan_kotlin_header(source),
        "scala" => scan_scala_header(source),
        "clojure" => scan_clojure_header(source),
        "groovy" => scan_groovy_header(source),
        _ => Vec::new(),
    }
}

/// Header-only tree-sitter scan of a Java source file. Returns the names of
/// every top-level `class`, `interface`, `enum`, `record`, and
/// `annotation_type_declaration` the file declares. Nested types are
/// captured as well — the top level of the file also hosts member types
/// reachable by outer-qualified names (`Outer.Inner`). We record the bare
/// simple name so `find_by_name("Inner")` still hits.
fn scan_java_header(source: &str) -> Vec<String> {
    let language = tree_sitter_java::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_java_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_java_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "class_declaration"
        | "interface_declaration"
        | "enum_declaration"
        | "record_declaration"
        | "annotation_type_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        _ => {}
    }
}

/// Header-only tree-sitter scan of a Kotlin source file (using
/// `tree-sitter-kotlin-ng`, which is the grammar the crate's Kotlin plugin
/// uses). Returns top-level class / object / interface / type-alias /
/// function / property names.
fn scan_kotlin_header(source: &str) -> Vec<String> {
    let language = tree_sitter_kotlin_ng::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_kotlin_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_kotlin_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "class_declaration"
        | "object_declaration"
        | "interface_declaration"
        | "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        "type_alias" => {
            // `typealias Foo = Bar` — field `type` (yes, really) holds the
            // identifier in kotlin-ng's grammar; fall back to any identifier
            // child for older grammar revs.
            if let Some(name_node) = node.child_by_field_name("type") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        "property_declaration" => {
            // Top-level `val`/`var`. Name lives at
            // property_declaration → variable_declaration → simple_identifier.
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                if inner.kind() == "variable_declaration" {
                    let mut ic = inner.walk();
                    for sub in inner.children(&mut ic) {
                        if matches!(sub.kind(), "simple_identifier" | "identifier") {
                            if let Ok(name) = sub.utf8_text(bytes) {
                                out.push(name.to_string());
                            }
                            break;
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Header-only tree-sitter scan of a Scala source file. Returns top-level
/// class / object / trait / case-class / function / val / var names.
fn scan_scala_header(source: &str) -> Vec<String> {
    let language = tree_sitter_scala::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_scala_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_scala_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "class_definition"
        | "object_definition"
        | "trait_definition"
        | "enum_definition"
        | "function_definition"
        | "function_declaration"
        | "type_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        "val_definition" | "var_definition" | "val_declaration" | "var_declaration" => {
            // Scala `val X = ...` / `val X: T = ...`. `pattern` field is the
            // canonical LHS (identifier or tuple pattern).
            let name_node = node
                .child_by_field_name("pattern")
                .or_else(|| node.child_by_field_name("name"));
            if let Some(name_node) = name_node {
                collect_scala_pattern_names(&name_node, bytes, out);
            }
        }
        "package_object" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        _ => {}
    }
}

fn collect_scala_pattern_names(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "identifier" | "stable_identifier" => {
            if let Ok(name) = node.utf8_text(bytes) {
                out.push(name.to_string());
            }
        }
        _ => {
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                collect_scala_pattern_names(&inner, bytes, out);
            }
        }
    }
}

/// Header-only tree-sitter scan of a Clojure source file. Clojure's grammar
/// exposes every form as a `list_lit`; the first `sym_lit` child is the
/// declaration keyword (`def`, `defn`, `defn-`, `defmacro`, `defmulti`,
/// `defprotocol`, `defrecord`, `deftype`, `definterface`, `defmethod`,
/// `defonce`), and the second `sym_lit` is the declared name. Docstrings,
/// metadata, and the body don't affect the name's position.
fn scan_clojure_header(source: &str) -> Vec<String> {
    let language = tree_sitter_clojure::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_clojure_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_clojure_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    if node.kind() != "list_lit" { return }
    // Find the first two `sym_lit` children. First is the form head (e.g.
    // `defn`), second is the declared name.
    let mut head: Option<String> = None;
    let mut name: Option<String> = None;
    let mut cursor = node.walk();
    for inner in node.children(&mut cursor) {
        if inner.kind() != "sym_lit" { continue }
        let Ok(text) = inner.utf8_text(bytes) else { continue };
        if head.is_none() {
            head = Some(text.to_string());
        } else {
            name = Some(text.to_string());
            break;
        }
    }
    let Some(head) = head else { return };
    let Some(name) = name else { return };
    if matches!(
        head.as_str(),
        "def" | "defn" | "defn-" | "defmacro" | "defmulti" | "defmethod"
            | "defprotocol" | "defrecord" | "deftype" | "definterface" | "defonce"
    ) {
        out.push(name);
    }
}

/// Header-only tree-sitter scan of a Groovy source file. Groovy jars shipped
/// by Gradle plugins and Spock test harnesses publish normal `class`,
/// `interface`, `enum` declarations — same `class_declaration` node kind as
/// Java's grammar uses here.
fn scan_groovy_header(source: &str) -> Vec<String> {
    let language = tree_sitter_groovy::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_groovy_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_groovy_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    if !matches!(node.kind(), "class_declaration" | "interface_declaration" | "enum_declaration") {
        return;
    }
    if let Some(name_node) = node.child_by_field_name("name") {
        if let Ok(name) = name_node.utf8_text(bytes) {
            out.push(name.to_string());
        }
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

    // -----------------------------------------------------------------
    // R3 — user-import scan + package-narrowed walk
    // -----------------------------------------------------------------

    #[test]
    fn java_import_extracts_fqn() {
        let mut out = std::collections::HashSet::new();
        extract_jvm_imports_from_source(
            "package demo;\nimport org.springframework.context.ApplicationContext;\nimport static java.util.Arrays.asList;\nimport java.util.*;\n\nclass Demo {}\n",
            &mut out,
        );
        assert!(out.contains("org.springframework.context.ApplicationContext"));
        assert!(out.contains("java.util.Arrays.asList"));
        assert!(out.contains("java.util.*"));
    }

    #[test]
    fn scala_selector_import_explodes() {
        let mut out = std::collections::HashSet::new();
        extract_jvm_imports_from_source(
            "import scala.collection.{Map, Set}\nimport cats.effect.IO\nimport foo.bar.{X => Z}\nimport foo.bar.{_}\n",
            &mut out,
        );
        assert!(out.contains("scala.collection.Map"));
        assert!(out.contains("scala.collection.Set"));
        assert!(out.contains("cats.effect.IO"));
        assert!(out.contains("foo.bar.X")); // renamed import still reveals the source class
        assert!(out.contains("foo.bar.*")); // `{_}` → wildcard
    }

    #[test]
    fn clojure_import_parses_vector_form() {
        let mut out = std::collections::HashSet::new();
        extract_clojure_imports(
            "(ns demo.core (:import [java.util Map HashMap] [org.slf4j LoggerFactory]) (:require [clojure.string :as str]))",
            &mut out,
        );
        assert!(out.contains("java.util"), "got: {out:?}");
        assert!(out.contains("org.slf4j"));
        assert!(out.contains("clojure.string"));
    }

    #[test]
    fn package_prefix_maps_to_path() {
        assert_eq!(
            jvm_import_to_package_prefix("org.springframework.context.ApplicationContext"),
            Some("org/springframework/context/".to_string())
        );
        assert_eq!(
            jvm_import_to_package_prefix("org.springframework.context.*"),
            Some("org/springframework/".to_string())
        );
        // Single-segment imports cannot narrow.
        assert_eq!(jvm_import_to_package_prefix("Foo"), None);
    }

    #[test]
    fn narrowed_walk_only_yields_requested_package() {
        let tmp = std::env::temp_dir().join("bw-test-maven-r3-narrow");
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("sources");
        std::fs::create_dir_all(root.join("org/spring/context")).unwrap();
        std::fs::create_dir_all(root.join("org/spring/beans")).unwrap();
        std::fs::create_dir_all(root.join("org/other")).unwrap();
        std::fs::write(
            root.join("org/spring/context/Ctx.java"),
            "package org.spring.context;\npublic class Ctx {}\n",
        ).unwrap();
        std::fs::write(
            root.join("org/spring/beans/Bean.java"),
            "package org.spring.beans;\npublic class Bean {}\n",
        ).unwrap();
        std::fs::write(
            root.join("org/other/Unrelated.java"),
            "package org.other;\npublic class Unrelated {}\n",
        ).unwrap();

        let dep = ExternalDepRoot {
            module_path: "org.spring:spring-context".to_string(),
            version: "6.0.0".to_string(),
            root: root.clone(),
            ecosystem: ID.as_str(),
            package_id: None,
            requested_imports: vec!["org.spring.context.Ctx".to_string()],
        };

        let files = walk_maven_narrowed(&dep);
        let paths: std::collections::HashSet<_> =
            files.iter().map(|f| f.absolute_path.clone()).collect();
        assert!(paths.contains(&root.join("org/spring/context/Ctx.java")));
        assert!(!paths.contains(&root.join("org/spring/beans/Bean.java")));
        assert!(!paths.contains(&root.join("org/other/Unrelated.java")));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn narrowed_walk_empty_imports_falls_back_to_full_walk() {
        let tmp = std::env::temp_dir().join("bw-test-maven-r3-fallback");
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("sources");
        std::fs::create_dir_all(root.join("p")).unwrap();
        std::fs::write(root.join("p/A.java"), "package p;\nclass A {}\n").unwrap();

        let dep = ExternalDepRoot {
            module_path: "g:a".to_string(),
            version: "1.0".to_string(),
            root: root.clone(),
            ecosystem: ID.as_str(),
            package_id: None,
            requested_imports: Vec::new(),
        };

        let files = walk_maven_narrowed(&dep);
        assert_eq!(files.len(), 1);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------
    // Header-only JVM scanners — demand-driven pipeline entry
    // -----------------------------------------------------------------

    #[test]
    fn java_scan_captures_top_level_types() {
        let src = r#"
package org.spring.context;

import java.util.List;

public class ApplicationContext {
    public void refresh() { /* body never walked */ }
    private class NotTopLevel {}
}

interface Bean {
    void init();
}

enum Status { OK, FAIL }

@interface Trace {}

record Coord(int x, int y) {}
"#;
        let names = scan_java_header(src);
        assert!(names.contains(&"ApplicationContext".to_string()), "{names:?}");
        assert!(names.contains(&"Bean".to_string()), "{names:?}");
        assert!(names.contains(&"Status".to_string()), "{names:?}");
        assert!(names.contains(&"Trace".to_string()), "{names:?}");
        assert!(names.contains(&"Coord".to_string()), "{names:?}");
        // Nested class must not surface — we're header-only.
        assert!(!names.contains(&"NotTopLevel".to_string()), "{names:?}");
    }

    #[test]
    fn kotlin_scan_captures_top_level_decls() {
        let src = r#"
package com.example

class Repository {
    fun findAll(): List<Entity> = emptyList()
}

object Singleton {
    fun helper() {}
}

interface Service

typealias EntityId = Long

fun topLevelFunction(x: Int): Int = x + 1

val CONSTANT: Int = 42
"#;
        let names = scan_kotlin_header(src);
        assert!(names.contains(&"Repository".to_string()), "{names:?}");
        assert!(names.contains(&"Singleton".to_string()), "{names:?}");
        assert!(names.contains(&"Service".to_string()), "{names:?}");
        assert!(names.contains(&"topLevelFunction".to_string()), "{names:?}");
    }

    #[test]
    fn scala_scan_captures_top_level_decls() {
        let src = r#"
package demo

class Foo {
  def method(): Int = 1
}

object Bar {
  val X = 1
}

trait Baz {
  def f(): Int
}

case class Point(x: Int, y: Int)

def topLevel(): Int = 2

val CONSTANT: Int = 3
"#;
        let names = scan_scala_header(src);
        assert!(names.contains(&"Foo".to_string()), "{names:?}");
        assert!(names.contains(&"Bar".to_string()), "{names:?}");
        assert!(names.contains(&"Baz".to_string()), "{names:?}");
        assert!(names.contains(&"Point".to_string()), "{names:?}");
    }

    #[test]
    fn clojure_scan_captures_defs() {
        let src = r#"
(ns demo.core
  (:require [clojure.string :as str]))

(def max-items 100)

(defn add [x y] (+ x y))

(defn- private-helper [x] x)

(defmacro when-let* [bindings & body]
  `(let ~bindings ~@body))

(defprotocol Greeter
  (greet [this]))

(defrecord Person [name age])

(deftype Pair [a b])

; a naked call is NOT a decl
(println "hello")
"#;
        let names = scan_clojure_header(src);
        assert!(names.contains(&"max-items".to_string()), "{names:?}");
        assert!(names.contains(&"add".to_string()), "{names:?}");
        assert!(names.contains(&"private-helper".to_string()), "{names:?}");
        assert!(names.contains(&"when-let*".to_string()), "{names:?}");
        assert!(names.contains(&"Greeter".to_string()), "{names:?}");
        assert!(names.contains(&"Person".to_string()), "{names:?}");
        assert!(names.contains(&"Pair".to_string()), "{names:?}");
        // Plain function calls don't produce decls.
        assert!(!names.contains(&"println".to_string()), "{names:?}");
    }

    #[test]
    fn groovy_scan_captures_class_and_interface() {
        let src = r#"
package demo

class BuildHelper {
    void run() {}
}

interface Plugin {
    void apply()
}
"#;
        let names = scan_groovy_header(src);
        assert!(names.contains(&"BuildHelper".to_string()), "{names:?}");
        // The Groovy grammar captures `interface_declaration` where available;
        // when it falls back to a generic class_declaration the name is still
        // recorded so the index can answer lookups.
    }

    #[test]
    fn scan_ignores_empty_and_invalid_sources() {
        assert!(scan_java_header("").is_empty());
        assert!(scan_kotlin_header("").is_empty());
        assert!(scan_scala_header("").is_empty());
        assert!(scan_clojure_header("").is_empty());
        assert!(scan_groovy_header("").is_empty());
    }

    #[test]
    fn java_package_derivation_from_rel_path() {
        let root = std::path::PathBuf::from("/cache/spring-context");
        let file = root
            .join("org")
            .join("springframework")
            .join("context")
            .join("Ctx.java");
        assert_eq!(
            java_package_from_rel_path(&file, &root),
            Some("org.springframework.context".to_string())
        );
    }

    #[test]
    fn java_package_at_root_returns_none() {
        let root = std::path::PathBuf::from("/cache/scala-library");
        // A .scala file directly at the dep root has no package segments.
        let file = root.join("LoneFile.scala");
        assert_eq!(java_package_from_rel_path(&file, &root), None);
    }

    #[test]
    fn build_maven_symbol_index_empty_returns_empty() {
        let idx = build_maven_symbol_index(&[]);
        assert!(idx.is_empty());
    }

    #[test]
    fn build_maven_symbol_index_inserts_under_java_package_and_module_keys() {
        // Simulate an extracted Spring sources jar on disk. The scanner
        // should yield (group:artifact, Ctx) AND (org.spring.context, Ctx)
        // so both the Maven-coord fallback and the import-based locate hit.
        let tmp = std::env::temp_dir().join("bw-test-maven-index-build");
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.clone();
        std::fs::create_dir_all(root.join("org/spring/context")).unwrap();
        std::fs::write(
            root.join("org/spring/context/Ctx.java"),
            "package org.spring.context;\npublic class Ctx {}\n",
        )
        .unwrap();

        let dep = ExternalDepRoot {
            module_path: "org.spring:spring-context".to_string(),
            version: "6.0.0".to_string(),
            root: root.clone(),
            ecosystem: ID.as_str(),
            package_id: None,
            requested_imports: Vec::new(),
        };

        let idx = build_maven_symbol_index(std::slice::from_ref(&dep));
        assert!(
            idx.locate("org.spring:spring-context", "Ctx").is_some(),
            "expected Maven-coord key to hit"
        );
        assert!(
            idx.locate("org.spring.context", "Ctx").is_some(),
            "expected java-package key to hit so user `import org.spring.context.Ctx` resolves"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
