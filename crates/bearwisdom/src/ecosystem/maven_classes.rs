// =============================================================================
// ecosystem/maven_classes.rs — demand-driven Java bytecode walker
//
// Maven/Gradle projects declare deps via pom.xml/build.gradle; the build
// downloads each dep's `.jar` (bytecode) into the local cache. The
// optional `-sources.jar` (containing `.java` sources) is opt-in — most
// projects don't fetch them. The `MavenEcosystem` source path walks
// sources-jars when present but is blind to the bytecode-only jars.
// This ecosystem fills that gap using a demand-driven approach:
//
//   1. `locate_roots` discovers bytecode jars via the same coordinate-driven
//      cache probe as before, returning one `ExternalDepRoot` per jar.
//   2. `build_symbol_index` scans each jar's ZIP central directory (cheap —
//      no bytecode parse) and registers every class name against a virtual
//      path of the form `ext:jar:<jar_abs>!<entry_name>`.
//   3. `uses_demand_driven_parse` returns `true`, so the eager
//      `parse_metadata_only` path is bypassed entirely.
//   4. The resolve engine's materialize-on-miss path cracks exactly the
//      one class file a ref demands, not the whole jar.
//
// This eliminates the 77-minute Grails-monorepo wedge that arose from
// cracking 311 k entries up front during the index-build pass.
// =============================================================================

use std::fs;
use std::path::{Path, PathBuf};

use tracing::debug;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, SymbolLocationIndex};
use crate::ecosystem::externals::{
    coursier_cache_root, gradle_caches_root, maven_local_repo, resolve_coursier_bytecode_jar,
    resolve_gradle_bytecode_jar, resolve_maven_artifact_dir, ExternalDepRoot,
    ExternalSourceLocator,
};
use crate::ecosystem::jar_walker;
use crate::ecosystem::manifest::maven::MavenCoord;
use crate::ecosystem::maven::{
    collect_declared_jvm_coords, collect_workspace_artifact_ids, jvm_sources_jar_available,
    ID as MAVEN_ID,
};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("maven-classes");
const ECOSYSTEM_TAG: &str = "maven-classes";
const LANGUAGES: &[&str] = &["java"];
/// Safety bound on jars per project. The coordinate-driven probe already
/// bounds the set to declared deps; this guards a manifest with a
/// pathological coordinate count.
const MAX_JARS_PER_PROJECT: usize = 300;
/// Cap per-jar size — anything bigger is almost certainly a fat shaded
/// artefact that would dominate index time without proportional value.
const MAX_JAR_BYTES: u64 = 32 * 1024 * 1024;

pub struct MavenClassesEcosystem;

impl Ecosystem for MavenClassesEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Package
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::TransitiveOn(MAVEN_ID)
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        jars_as_dep_roots(ctx.project_root)
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    /// Build the `(module, class_name) → virtual_path` index by scanning each
    /// jar's ZIP central directory. Cheap: reads only the directory index, never
    /// decompresses any class file. The virtual path encodes the jar + entry so
    /// the materialize path can crack exactly that one class on demand.
    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        let mut index = SymbolLocationIndex::new();
        for dep in dep_roots {
            let jar_path = &dep.root;
            let archive_str = jar_path.to_string_lossy().replace('\\', "/");
            let entries = jar_walker::list_jar_class_entries(jar_path);
            let count = entries.len();
            for (entry_name, class_name) in entries {
                let virt = PathBuf::from(format!("ext:jar:{archive_str}!{entry_name}"));
                index.insert(dep.module_path.clone(), class_name, virt);
            }
            debug!(
                "maven-classes: indexed {} class entries from {}",
                count,
                jar_path.display()
            );
        }
        index
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }
}

impl ExternalSourceLocator for MavenClassesEcosystem {
    fn ecosystem(&self) -> &'static str {
        ECOSYSTEM_TAG
    }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        jars_as_dep_roots(project_root)
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    // parse_metadata_only returns None: the demand-driven path handles all
    // class extraction. Returning None here prevents the eager full-jar dump
    // that previously wedged the indexer on large JVM projects.
}

/// Return one `ExternalDepRoot` per bytecode jar the project declares.
/// Each root's `root` field is the jar path; `module_path` is the
/// `artifact_id` used as the module key in `build_symbol_index`.
fn jars_as_dep_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
    jars_as_dep_roots_in_caches(
        project_root,
        maven_local_repo().as_deref(),
        gradle_caches_root().as_deref(),
        coursier_cache_root().as_deref(),
    )
}

/// Core of `jars_as_dep_roots` with explicit cache roots so tests can
/// exercise the probe against fixture caches without touching process-global
/// cache-discovery env vars.
fn jars_as_dep_roots_in_caches(
    project_root: &Path,
    m2: Option<&Path>,
    gradle: Option<&Path>,
    coursier: Option<&Path>,
) -> Vec<ExternalDepRoot> {
    let mut out: Vec<ExternalDepRoot> = Vec::new();

    // Project-local `lib/`, `libs/`, `vendor/`, `deps/` — common in older
    // projects that check jars into the repo. Scoped to the project tree.
    let mut local_jars: Vec<PathBuf> = Vec::new();
    for sub in &["lib", "libs", "vendor", "deps"] {
        let dir = project_root.join(sub);
        if dir.is_dir() {
            collect_local_jars(&dir, &mut local_jars, 0);
        }
    }
    for jar in local_jars {
        let module = jar
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        out.push(ExternalDepRoot {
            module_path: module,
            version: String::new(),
            root: jar,
            ecosystem: ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }

    // The workspace's own module ids resolve to project build output, not
    // cached jars — exclude them from the bytecode probe.
    let own_modules = collect_workspace_artifact_ids(project_root);
    let mut count = out.len();

    for coord in collect_declared_jvm_coords(project_root) {
        if count >= MAX_JARS_PER_PROJECT {
            break;
        }
        if own_modules.contains(&coord.artifact_id) {
            continue;
        }
        // The source-jar path already covers any coordinate with a cached
        // `-sources.jar`; cracking its bytecode here would double-index.
        if jvm_sources_jar_available(m2, gradle, coursier, &coord) {
            continue;
        }
        if let Some(jar) = resolve_jvm_bytecode_jar(m2, gradle, coursier, &coord) {
            if let Ok(meta) = fs::metadata(&jar) {
                if meta.len() > MAX_JAR_BYTES {
                    continue;
                }
            }
            out.push(ExternalDepRoot {
                module_path: coord.artifact_id.clone(),
                version: coord.version.clone().unwrap_or_default(),
                root: jar,
                ecosystem: ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
            count += 1;
        }
    }
    out
}

/// Probe the `.m2`, Gradle, and Coursier caches (in that precedence order)
/// for `coord`'s bytecode jar. First hit wins; `None` when the coordinate
/// isn't cached in any layout. Honors `coord.version`, falling back to the
/// largest cached version when absent or dynamic.
///
/// When the manifest-pinned version isn't cached, a second pass retries with
/// the version dropped so a close-enough cached version still resolves. This
/// mirrors the sources-path fallback in `resolve_and_push_jvm`: a project
/// often pins a version the local cache doesn't hold verbatim (the compile
/// classpath resolved a different one), and a near version's bytecode is far
/// more useful than no externals at all.
fn resolve_jvm_bytecode_jar(
    m2: Option<&Path>,
    gradle: Option<&Path>,
    coursier: Option<&Path>,
    coord: &MavenCoord,
) -> Option<PathBuf> {
    if let Some(jar) = resolve_jvm_bytecode_jar_pinned(m2, gradle, coursier, coord) {
        return Some(jar);
    }
    if coord.version.is_some() {
        let unpinned = MavenCoord {
            group_id: coord.group_id.clone(),
            artifact_id: coord.artifact_id.clone(),
            version: None,
        };
        return resolve_jvm_bytecode_jar_pinned(m2, gradle, coursier, &unpinned);
    }
    None
}

/// Single-pass probe across the three caches for `coord` exactly as given —
/// the pinned version when set, each cache's newest-wins fallback when None.
fn resolve_jvm_bytecode_jar_pinned(
    m2: Option<&Path>,
    gradle: Option<&Path>,
    coursier: Option<&Path>,
    coord: &MavenCoord,
) -> Option<PathBuf> {
    if let Some(repo) = m2 {
        if let Some((version, artifact_dir)) = resolve_maven_artifact_dir(repo, coord) {
            let jar = artifact_dir.join(format!("{}-{}.jar", coord.artifact_id, version));
            if jar.is_file() {
                return Some(jar);
            }
        }
    }
    if let Some(cache) = gradle {
        if let Some(jar) = resolve_gradle_bytecode_jar(cache, coord) {
            return Some(jar);
        }
    }
    if let Some(cache) = coursier {
        if let Some(jar) = resolve_coursier_bytecode_jar(cache, coord) {
            return Some(jar);
        }
    }
    None
}

/// Recursively collect `.jar` files under a project-local directory,
/// excluding the `-sources`/`-javadoc`/`-tests` classifier variants.
fn collect_local_jars(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    if depth > 10 || out.len() > MAX_JARS_PER_PROJECT {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() > MAX_JARS_PER_PROJECT {
            return;
        }
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            collect_local_jars(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".jar") {
                continue;
            }
            if name.ends_with("-sources.jar")
                || name.ends_with("-javadoc.jar")
                || name.ends_with("-tests.jar")
            {
                continue;
            }
            out.push(path);
        }
    }
}

#[cfg(test)]
pub(super) fn _test_discover_jars_in_caches(
    project_root: &Path,
    m2: Option<&Path>,
    gradle: Option<&Path>,
    coursier: Option<&Path>,
) -> Vec<PathBuf> {
    jars_as_dep_roots_in_caches(project_root, m2, gradle, coursier)
        .into_iter()
        .map(|dep| dep.root)
        .collect()
}

#[cfg(test)]
pub(super) fn _test_resolve_jvm_bytecode_jar(
    m2: Option<&Path>,
    gradle: Option<&Path>,
    coursier: Option<&Path>,
    coord: &MavenCoord,
) -> Option<PathBuf> {
    resolve_jvm_bytecode_jar(m2, gradle, coursier, coord)
}

#[cfg(test)]
#[path = "maven_classes_tests.rs"]
mod tests;
