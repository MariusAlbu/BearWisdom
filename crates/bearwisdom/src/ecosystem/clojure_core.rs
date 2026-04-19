// =============================================================================
// ecosystem/clojure_core.rs — clojure.core / clojure.lang (stdlib ecosystem)
//
// Probes maven-resolved `org.clojure:clojure` sources jar in ~/.m2. The
// resulting tree contains .clj (core.clj, set.clj, string.clj ...) and
// .java files for `clojure.lang.*` interop types. The generic JVM walker
// tags each file correctly by extension.
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

pub const ID: EcosystemId = EcosystemId::new("clojure-core");
const LEGACY_ECOSYSTEM_TAG: &str = "clojure-core";
const LANGUAGES: &[&str] = &["clojure"];

pub struct ClojureCoreEcosystem;

impl Ecosystem for ClojureCoreEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("clojure")
    }
    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> { discover() }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(
        &self,
        dep_roots: &[crate::ecosystem::externals::ExternalDepRoot],
    ) -> crate::ecosystem::symbol_index::SymbolLocationIndex {
        // Two-source index: maven builder covers `clojure.lang.*` Java
        // interop types (headers-only tree-sitter parse); regex scanner
        // below covers `(defn/def/defmacro ... name)` in `.clj` files.
        let mut idx = super::maven::build_maven_symbol_index(dep_roots);
        for dep in dep_roots {
            for wf in super::maven::walk_generic_jvm_root(dep) {
                if !wf.relative_path.ends_with(".clj")
                    && !wf.relative_path.ends_with(".cljs")
                    && !wf.relative_path.ends_with(".cljc")
                {
                    continue;
                }
                let Ok(source) = std::fs::read_to_string(&wf.absolute_path) else { continue };
                scan_clojure_defs(&source, &wf, &mut idx);
            }
        }
        idx
    }
}

/// Emit `(module, name) → file` entries for Clojure top-level definitions.
/// Module is the `(ns …)` declaration text from the file (or filename stem
/// when ns is absent). Recognized forms: `defn`, `defn-`, `def`, `defmacro`,
/// `defmulti`, `defmethod`, `defprotocol`, `defrecord`, `deftype`.
fn scan_clojure_defs(
    source: &str,
    wf: &WalkedFile,
    idx: &mut crate::ecosystem::symbol_index::SymbolLocationIndex,
) {
    let re_ns = regex::Regex::new(r"\(ns\s+([A-Za-z0-9._\-]+)").expect("clj ns regex");
    let re_def = regex::Regex::new(
        r"\((?:defn-?|def|defmacro|defmulti|defmethod|defprotocol|defrecord|deftype)\s+([A-Za-z0-9._*+!?<>=/\-]+)",
    )
    .expect("clj def regex");

    let ns_name = re_ns
        .captures(source)
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| {
            std::path::Path::new(&wf.relative_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("clojure.core")
                .to_string()
        });

    for cap in re_def.captures_iter(source) {
        let name = cap[1].to_string();
        idx.insert(&ns_name, &name, &wf.relative_path);
    }
}

impl ExternalSourceLocator for ClojureCoreEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> { discover() }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }
}

fn discover() -> Vec<ExternalDepRoot> {
    let Some(repo) = maven_local_repo() else { return Vec::new() };
    let artifact_dir = repo.join("org").join("clojure").join("clojure");
    if !artifact_dir.is_dir() { return Vec::new() }
    let Ok(versions) = std::fs::read_dir(&artifact_dir) else { return Vec::new() };
    let mut vs: Vec<PathBuf> = versions
        .flatten().filter(|e| e.path().is_dir()).map(|e| e.path()).collect();
    vs.sort();
    let Some(latest) = vs.into_iter().next_back() else { return Vec::new() };
    let Ok(files) = std::fs::read_dir(&latest) else { return Vec::new() };
    let jar = files.flatten().find(|e| {
        e.path().file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("-sources.jar"))
    }).map(|e| e.path());
    let Some(jar_path) = jar else { return Vec::new() };

    let cache_base = repo.parent().unwrap_or(&repo).join("bearwisdom-clojure-core-cache");
    let _ = std::fs::create_dir_all(&cache_base);
    let stem = jar_path.file_name().and_then(|n| n.to_str())
        .map(|n| n.trim_end_matches(".jar").to_string())
        .unwrap_or_else(|| "clojure-sources".to_string());
    let cache_dir = cache_base.join(stem);
    if !cache_dir.exists() || is_cache_stale(&jar_path, &cache_dir) {
        if let Err(e) = extract_java_sources_jar(&jar_path, &cache_dir) {
            debug!("Failed to extract {}: {e}", jar_path.display());
            return Vec::new();
        }
    }
    vec![ExternalDepRoot {
        module_path: "org.clojure:clojure".to_string(),
        version: latest.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string(),
        root: cache_dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<ClojureCoreEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(ClojureCoreEcosystem)).clone()
}
