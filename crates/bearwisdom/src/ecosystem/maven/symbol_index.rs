// =============================================================================
// ecosystem/maven/symbol_index.rs
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
    is_cache_stale, maven_local_repo, resolve_coursier_sources_jar,
    resolve_coursier_submodule_jars, resolve_gradle_sources_jar, resolve_maven_artifact_dir,
    ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH,
};
use crate::ecosystem::manifest::maven::{parse_pom_xml_coords, MavenCoord};
use crate::ecosystem::manifest::{
    clojure as clojure_manifest, gradle as gradle_manifest, sbt as sbt_manifest,
};
use crate::walker::WalkedFile;

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
        for wf in super::walk_maven_root(dep) {
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
            let mut rows: Vec<(String, String, PathBuf)> = Vec::with_capacity(names.len() * 2);
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
pub(crate) fn java_package_from_rel_path(file: &Path, dep_root: &Path) -> Option<String> {
    let rel = file.strip_prefix(dep_root).ok()?;
    let mut segs: Vec<&str> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    if segs.len() < 2 {
        return None;
    }
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
pub(crate) fn scan_java_header(source: &str) -> Vec<String> {
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

pub(crate) fn collect_java_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
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
pub(crate) fn scan_kotlin_header(source: &str) -> Vec<String> {
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

pub(crate) fn collect_kotlin_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
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
pub(crate) fn scan_scala_header(source: &str) -> Vec<String> {
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

pub(crate) fn collect_scala_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
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

pub(crate) fn collect_scala_pattern_names(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
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
pub(crate) fn scan_clojure_header(source: &str) -> Vec<String> {
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

pub(crate) fn collect_clojure_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    if node.kind() != "list_lit" {
        return;
    }
    // Find the first two `sym_lit` children. First is the form head (e.g.
    // `defn`), second is the declared name.
    let mut head: Option<String> = None;
    let mut name: Option<String> = None;
    let mut cursor = node.walk();
    for inner in node.children(&mut cursor) {
        if inner.kind() != "sym_lit" {
            continue;
        }
        let Ok(text) = inner.utf8_text(bytes) else {
            continue;
        };
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
        "def"
            | "defn"
            | "defn-"
            | "defmacro"
            | "defmulti"
            | "defmethod"
            | "defprotocol"
            | "defrecord"
            | "deftype"
            | "definterface"
            | "defonce"
    ) {
        out.push(name);
    }
}

/// Header-only tree-sitter scan of a Groovy source file. Groovy jars shipped
/// by Gradle plugins and Spock test harnesses publish normal `class`,
/// `interface`, `enum` declarations — same `class_declaration` node kind as
/// Java's grammar uses here.
pub(crate) fn scan_groovy_header(source: &str) -> Vec<String> {
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

pub(crate) fn collect_groovy_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    if !matches!(
        node.kind(),
        "class_declaration" | "interface_declaration" | "enum_declaration"
    ) {
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
