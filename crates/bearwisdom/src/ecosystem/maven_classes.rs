// =============================================================================
// ecosystem/maven_classes.rs — Java bytecode walker for transitive jars
//
// Maven/Gradle projects declare deps via pom.xml/build.gradle; the build
// downloads each dep's `.jar` (bytecode) into the local cache. The
// optional `-sources.jar` (containing `.java` sources) is opt-in — most
// projects don't fetch them. The existing `MavenEcosystem` walks
// sources-jars when present but is blind to the bytecode-only jars.
// This walker fills that gap: when a declared dep has a `.jar` in the
// cache and no matching `-sources.jar`, crack the bytecode and emit
// public/protected types + members as ParsedFile entries.
//
// **Activation:** transitive on Maven — if Maven didn't already fire,
// neither did source-jar discovery, so there's nothing for us to fill
// in.
//
// **Performance:** bounded — only parses jars whose group:artifact
// appears in `ctx.manifests[Maven|Gradle]`. Skips signed jars
// (META-INF/MANIFEST.MF), java agent jars, and anything > 32 MB
// (rare-but-possible bytecode artefacts that bog down the parser).
// =============================================================================

use std::fs;
use std::path::{Path, PathBuf};

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::jar_walker;
use crate::ecosystem::maven::{ID as MAVEN_ID};
use crate::types::ParsedFile;
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("maven-classes");
const ECOSYSTEM_TAG: &str = "maven-classes";
const LANGUAGES: &[&str] = &["java"];
/// Cap per-jar parse cost — anything bigger is almost certainly a fat
/// shaded artefact that'd dominate index time without proportional
/// resolution value.
const MAX_JAR_BYTES: u64 = 32 * 1024 * 1024;
/// Cap on jars cracked per project. Real projects pull dozens of jars;
/// this is a safety net for misconfigured caches.
const MAX_JARS_PER_PROJECT: usize = 300;

pub struct MavenClassesEcosystem;

impl Ecosystem for MavenClassesEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::TransitiveOn(MAVEN_ID)
    }

    fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        // No on-disk root layout — everything happens via parse_metadata_only.
        Vec::new()
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }
}

impl ExternalSourceLocator for MavenClassesEcosystem {
    fn ecosystem(&self) -> &'static str { ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        Vec::new()
    }
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn parse_metadata_only(&self, project_root: &Path) -> Option<Vec<ParsedFile>> {
        let jars = discover_jars(project_root);
        if jars.is_empty() { return None }
        let mut out: Vec<ParsedFile> = Vec::new();
        for (i, jar) in jars.into_iter().enumerate() {
            if i >= MAX_JARS_PER_PROJECT { break }
            if let Ok(meta) = fs::metadata(&jar) {
                if meta.len() > MAX_JAR_BYTES { continue }
            }
            out.extend(jar_walker::walk_jar(&jar));
        }
        if out.is_empty() { None } else { Some(out) }
    }
}

/// Find `.jar` files (excluding `-sources.jar`/`-javadoc.jar`/`-tests.jar`)
/// that look like they belong to deps the project might consume. Probes
/// the standard maven/gradle/coursier cache layouts. Best-effort — no
/// dependency-graph traversal, just on-disk enumeration.
fn discover_jars(project_root: &Path) -> Vec<PathBuf> {
    let mut jars = Vec::new();
    // Project-local `lib/`, `libs/`, `vendor/` — common in older projects.
    for sub in &["lib", "libs", "vendor", "deps"] {
        let dir = project_root.join(sub);
        if dir.is_dir() {
            collect_jars(&dir, &mut jars, 0);
        }
    }
    // Standard cache locations.
    if let Some(home) = dirs_home() {
        let m2 = home.join(".m2").join("repository");
        if m2.is_dir() { collect_jars(&m2, &mut jars, 0); }
        let gradle = home.join(".gradle").join("caches").join("modules-2").join("files-2.1");
        if gradle.is_dir() { collect_jars(&gradle, &mut jars, 0); }
        let coursier = home.join("AppData").join("Local").join("Coursier").join("cache").join("v1");
        if coursier.is_dir() { collect_jars(&coursier, &mut jars, 0); }
    }
    jars
}

fn collect_jars(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    if depth > 10 || out.len() > MAX_JARS_PER_PROJECT { return }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if out.len() > MAX_JARS_PER_PROJECT { return }
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            collect_jars(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".jar") { continue }
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

fn dirs_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let p = PathBuf::from(home);
        if p.is_dir() { return Some(p) }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home);
        if p.is_dir() { return Some(p) }
    }
    None
}
