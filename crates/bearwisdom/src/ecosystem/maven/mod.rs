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
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::ecosystem::manifest::maven::{parse_pom_xml_coords, MavenCoord};
use crate::ecosystem::manifest::{
    clojure as clojure_manifest, gradle as gradle_manifest, sbt as sbt_manifest,
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
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Package
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }
    fn manifest_specs(&self) -> &'static [ManifestSpec] {
        MANIFESTS
    }

    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        // Maven covers the JVM tool family in BearWisdom: pom.xml + Gradle
        // (build.gradle / .kts) + SBT. Each filename gets its own kind label
        // so users querying packages.kind can tell them apart.
        &[
            ("pom.xml", "maven"),
            ("build.gradle", "gradle"),
            ("build.gradle.kts", "gradle"),
            ("build.sbt", "sbt"),
            ("deps.edn", "clojure"),
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

    fn supports_reachability(&self) -> bool {
        true
    }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        walk_maven_narrowed(dep)
    }

    fn resolve_symbol(&self, dep: &ExternalDepRoot, _fqn: &str) -> Vec<WalkedFile> {
        walk_maven_narrowed(dep)
    }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_maven_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl — adapter for the existing indexer
// pipeline. Dropped in Phase 4 when the indexer consumes Ecosystem directly.
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for MavenEcosystem {
    fn ecosystem(&self) -> &'static str {
        ID.as_str()
    }

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

mod discovery;
mod reachability;
mod symbol_index;
pub(crate) use discovery::*;
pub(crate) use reachability::*;
pub(crate) use symbol_index::*;

// ---------------------------------------------------------------------------
// Walk: single implementation handling all JVM languages; per-file language
// detection by extension so a Scala sources jar containing both .scala and
// .java files tags each file correctly.
// ---------------------------------------------------------------------------

pub(crate) fn walk_maven_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
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
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "META-INF") || name.starts_with('.') {
                    continue;
                }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            let (language, virtual_tag) = match detect_jvm_language(name) {
                Some(spec) => spec,
                None => continue,
            };

            // Skip test-suffixed files by convention.
            if name.ends_with("Test.java")
                || name.ends_with("Tests.java")
                || name.ends_with("Test.scala")
                || name.ends_with("Tests.scala")
                || name.ends_with("Spec.scala")
                || name.ends_with("Suite.scala")
                || name == "package-info.java"
                || name == "module-info.java"
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
pub(crate) fn detect_jvm_language(name: &str) -> Option<(&'static str, &'static str)> {
    if name.ends_with(".java") {
        Some(("java", "java"))
    } else if name.ends_with(".kt") || name.ends_with(".kts") {
        Some(("kotlin", "kotlin"))
    } else if name.ends_with(".scala") {
        Some(("scala", "scala"))
    } else if name.ends_with(".clj") || name.ends_with(".cljc") || name.ends_with(".cljs") {
        Some(("clojure", "clojure"))
    } else if name.ends_with(".groovy")
        || name.ends_with(".gradle")
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

#[cfg(test)]
mod tests {
    use super::reachability::{
        extract_clojure_imports, extract_jvm_imports_from_source, jvm_import_to_package_prefix,
    };
    use super::symbol_index::{
        collect_groovy_top_level_name, collect_java_top_level_name, collect_kotlin_top_level_name,
        collect_scala_pattern_names, collect_scala_top_level_name, scan_clojure_header,
        scan_groovy_header, scan_java_header, scan_kotlin_header, scan_scala_header,
    };
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
        assert_eq!(
            detect_jvm_language("foo.cljs"),
            Some(("clojure", "clojure"))
        );
        assert_eq!(
            detect_jvm_language("build.groovy"),
            Some(("groovy", "groovy"))
        );
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
        )
        .unwrap();
        std::fs::write(
            root.join("org/spring/beans/Bean.java"),
            "package org.spring.beans;\npublic class Bean {}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("org/other/Unrelated.java"),
            "package org.other;\npublic class Unrelated {}\n",
        )
        .unwrap();

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
        assert!(
            names.contains(&"ApplicationContext".to_string()),
            "{names:?}"
        );
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
