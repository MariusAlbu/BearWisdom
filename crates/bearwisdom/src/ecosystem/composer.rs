// =============================================================================
// ecosystem/composer.rs — Composer ecosystem (PHP)
//
// Phase 2 + 3: consolidates `indexer/externals/php.rs` +
// `indexer/manifest/composer.rs`. Packages live at `vendor/<vendor>/<name>/`.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::debug;
use tree_sitter::{Node, Parser};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("composer");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["php"];
const LEGACY_ECOSYSTEM_TAG: &str = "php";

pub struct ComposerEcosystem;

impl Ecosystem for ComposerEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("php"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_php_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_php_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        walk_php_narrowed(dep)
    }

    fn resolve_symbol(
        &self,
        dep: &ExternalDepRoot,
        _fqn: &str,
    ) -> Vec<WalkedFile> {
        walk_php_narrowed(dep)
    }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_php_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for ComposerEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_php_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_php_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<ComposerEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(ComposerEcosystem)).clone()
}

// ===========================================================================
// Manifest reader
// ===========================================================================

pub struct ComposerManifest;

impl ManifestReader for ComposerManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Composer }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let entries = self.read_all(project_root);
        if entries.is_empty() { return None }
        let mut data = ManifestData::default();
        for e in &entries {
            data.dependencies.extend(e.data.dependencies.iter().cloned());
        }
        Some(data)
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let mut paths = Vec::new();
        collect_composer_files(project_root, &mut paths, 0);
        let mut out = Vec::new();
        for manifest_path in paths {
            let Ok(content) = std::fs::read_to_string(&manifest_path) else { continue };
            let mut data = ManifestData::default();
            let (name, deps) = parse_composer_json(&content);
            for pkg in deps { data.dependencies.insert(pkg); }
            let package_dir = manifest_path
                .parent().map(|p| p.to_path_buf())
                .unwrap_or_else(|| project_root.to_path_buf());
            out.push(ReaderEntry { package_dir, manifest_path, data, name });
        }
        out
    }
}

fn collect_composer_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | "vendor" | "node_modules" | "target" | "bin" | "obj"
            ) { continue }
            collect_composer_files(&path, out, depth + 1);
        } else if entry.file_name() == "composer.json" {
            out.push(path);
        }
    }
}

fn parse_composer_json(content: &str) -> (Option<String>, Vec<String>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return (None, Vec::new());
    };
    let Some(obj) = value.as_object() else { return (None, Vec::new()) };
    let name = obj.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
    let mut packages = Vec::new();
    for key in &["require", "require-dev"] {
        if let Some(serde_json::Value::Object(deps)) = obj.get(*key) {
            for pkg_name in deps.keys() {
                if pkg_name == "php"
                    || pkg_name.starts_with("ext-")
                    || pkg_name.starts_with("lib-")
                { continue }
                if !pkg_name.is_empty() { packages.push(pkg_name.clone()) }
            }
        }
    }
    (name, packages)
}

pub fn parse_composer_json_deps(content: &str) -> Vec<String> {
    parse_composer_json(content).1
}

// ===========================================================================
// Discovery + walk
// ===========================================================================

pub fn discover_php_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let composer_path = project_root.join("composer.json");
    if !composer_path.is_file() { return Vec::new() }
    let Ok(content) = std::fs::read_to_string(&composer_path) else { return Vec::new() };
    let declared = parse_composer_json_deps(&content);
    if declared.is_empty() { return Vec::new() }

    let vendor = project_root.join("vendor");
    if !vendor.is_dir() { return Vec::new() }

    // R3: collect every `use X\Y\Z;` statement from project PHP code once.
    // Each vendor package's dep root carries the full set; walk_php_narrowed
    // filters to files whose path matches one of these FQNs.
    let user_uses: Vec<String> = collect_php_user_uses(project_root)
        .into_iter()
        .collect();

    let mut roots = Vec::new();
    for dep in &declared {
        let pkg_dir = vendor.join(dep.replace('/', std::path::MAIN_SEPARATOR_STR));
        if pkg_dir.is_dir() {
            let version = read_composer_version(&pkg_dir);
            roots.push(ExternalDepRoot {
                module_path: dep.clone(),
                version,
                root: pkg_dir,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: user_uses.clone(),
            });
        }
    }
    debug!("PHP: {} external package roots", roots.len());
    roots
}

fn read_composer_version(pkg_dir: &Path) -> String {
    let installed = pkg_dir.join("composer.json");
    if let Ok(content) = std::fs::read_to_string(&installed) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(v) = val.get("version").and_then(|v| v.as_str()) {
                return v.to_string();
            }
        }
    }
    String::new()
}

fn walk_php_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    let walk_root = if dep.root.join("src").is_dir() {
        dep.root.join("src")
    } else {
        dep.root.clone()
    };
    walk_dir_bounded(&walk_root, &dep.root, dep, &mut out, 0);
    out
}

// ---------------------------------------------------------------------------
// R3 reachability — scan project `use` statements, narrow to referenced files
// ---------------------------------------------------------------------------
//
// PHP references external classes via `use Foo\Bar\Baz;` statements. We
// scan every .php file in the project, extract FQNs from `use` lines, and
// carry the full set on each vendor package's ExternalDepRoot. walk_php_narrowed
// keeps only files whose path ends with a matching tail (`Bar/Baz.php`),
// collapsing typical monoliths like symfony/http-foundation (~200 files)
// down to 5-20 files the app actually references.

fn collect_php_user_uses(project_root: &Path) -> std::collections::HashSet<String> {
    let mut uses = std::collections::HashSet::new();
    scan_php_uses_recursive(project_root, &mut uses, 0);
    uses
}

fn scan_php_uses_recursive(
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
                    ".git" | "vendor" | "node_modules" | "storage" | "bootstrap"
                        | "public" | "var" | "tmp" | "cache" | "build"
                ) || name.starts_with('.') { continue }
            }
            scan_php_uses_recursive(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".php") { continue }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            extract_php_uses_from_source(&content, out);
        }
    }
}

/// Parse `use Foo\Bar\Baz;` / `use Foo\Bar\{Baz, Qux};` / `use Foo\Bar as B;` /
/// `use function Foo\bar;` / `use const Foo\X;`. Scala-like brace blocks (PHP
/// group use declaration) are exploded into individual FQNs.
fn extract_php_uses_from_source(
    content: &str,
    out: &mut std::collections::HashSet<String>,
) {
    for raw in content.lines() {
        let line = raw.trim();
        let rest = match line.strip_prefix("use ") {
            Some(r) => r,
            None => continue,
        };
        // Strip trailing `;` and inline comment.
        let rest = rest.split(';').next().unwrap_or("").trim();
        let rest = rest.trim_start_matches("function ")
            .trim_start_matches("const ")
            .trim();
        if rest.is_empty() { continue }

        // Group use: `Foo\Bar\{Baz, Qux as Q, SubNs\Thing}`
        if let Some(brace_open) = rest.find('{') {
            if let Some(brace_close) = rest.find('}') {
                let prefix = rest[..brace_open].trim_end_matches('\\').trim();
                if prefix.is_empty() { continue }
                let inner = &rest[brace_open + 1..brace_close];
                for sel in inner.split(',') {
                    let sel = sel.trim();
                    let head = sel.split(" as ").next().unwrap_or("").trim();
                    if head.is_empty() { continue }
                    out.insert(format!("{prefix}\\{head}"));
                }
                continue;
            }
        }

        // Single use: strip `as Alias`.
        let head = rest.split(" as ").next().unwrap_or("").trim();
        if head.is_empty() { continue }
        out.insert(head.trim_start_matches('\\').to_string());
    }
}

/// Convert a PHP FQN like `Symfony\Component\HttpFoundation\Request` to a
/// suffix path `HttpFoundation/Request.php` that the narrowing walk matches
/// against. Using the last two segments (namespace/class) drops most
/// false-positive hits on common names like `Request` / `Response` without
/// needing to parse each package's composer autoload PSR-4 map.
fn php_fqn_to_path_suffix(fqn: &str) -> Option<String> {
    let cleaned = fqn.trim().trim_start_matches('\\');
    if cleaned.is_empty() { return None }
    let parts: Vec<&str> = cleaned.split('\\').filter(|p| !p.is_empty()).collect();
    match parts.len() {
        0 => None,
        1 => Some(format!("{}.php", parts[0])),
        _ => {
            let n = parts.len();
            Some(format!("{}/{}.php", parts[n - 2], parts[n - 1]))
        }
    }
}

fn walk_php_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() {
        return walk_php_root(dep);
    }
    let suffixes: std::collections::HashSet<String> = dep
        .requested_imports
        .iter()
        .filter_map(|fqn| php_fqn_to_path_suffix(fqn))
        .collect();
    if suffixes.is_empty() {
        return walk_php_root(dep);
    }

    let mut out = Vec::new();
    let walk_root = if dep.root.join("src").is_dir() {
        dep.root.join("src")
    } else {
        dep.root.clone()
    };
    walk_narrowed_dir(&walk_root, &dep.root, dep, &suffixes, &mut out, 0);
    out
}

fn walk_narrowed_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    suffixes: &std::collections::HashSet<String>,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut dir_files: Vec<(PathBuf, String)> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();
    let mut any_match = false;

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "test" | "Tests" | "Test" | "vendor" | "docs" | "examples")
                    || name.starts_with('.')
                { continue }
            }
            subdirs.push(path);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".php") { continue }
            if name.ends_with("Test.php") || name.ends_with("Tests.php") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if suffixes.iter().any(|s| rel_sub.ends_with(s)) {
                any_match = true;
            }
            dir_files.push((path, rel_sub));
        }
    }

    // Sibling inclusion: PHP's namespace-local refs (extends/implements/same-ns
    // types) don't need a `use` statement, so any dir with at least one matched
    // file probably owns types the matched class references. Emit every .php
    // file in that dir when we have a match; skip the whole dir when we don't.
    if any_match {
        for (path, rel_sub) in dir_files {
            out.push(WalkedFile {
                relative_path: format!("ext:php:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "php",
            });
        }
    }

    for sub in subdirs {
        walk_narrowed_dir(&sub, root, dep, suffixes, out, depth + 1);
    }
}

fn walk_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "test" | "Tests" | "Test" | "vendor" | "docs" | "examples")
                    || name.starts_with('.')
                { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".php") { continue }
            if name.ends_with("Test.php") || name.ends_with("Tests.php") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:php:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "php",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------

pub(crate) fn build_php_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_php_root(dep) {
            work.push((dep.module_path.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }
    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_php_header(&src)
                .into_iter()
                .map(|name| (module.clone(), name, wf.absolute_path.clone()))
                .collect()
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

/// Header-only tree-sitter scan of a PHP source file. Records top-level
/// class / interface / trait / enum / function / const names. Nested
/// class methods are left to the regular extractor when the file is
/// eventually pulled into the index.
fn scan_php_header(source: &str) -> Vec<String> {
    let language = tree_sitter_php::LANGUAGE_PHP.into();
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
    walk_php_decls(&root, bytes, &mut out, 0);
    out
}

fn walk_php_decls(node: &Node, bytes: &[u8], out: &mut Vec<String>, depth: u32) {
    if depth > 4 { return }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration"
            | "interface_declaration"
            | "trait_declaration"
            | "enum_declaration"
            | "function_definition"
            | "const_declaration" => {
                if let Some(name_node) = child
                    .child_by_field_name("name")
                    .or_else(|| find_first_name_child(&child, "name"))
                {
                    if let Ok(t) = name_node.utf8_text(bytes) {
                        out.push(t.to_string());
                    }
                }
            }
            "namespace_definition" | "namespace_use_declaration"
            | "program" | "compound_statement" => {
                // Recurse — namespaces wrap their contents in a block.
                walk_php_decls(&child, bytes, out, depth + 1);
            }
            _ => {}
        }
    }
}

fn find_first_name_child<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind { return Some(child) }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let c = ComposerEcosystem;
        assert_eq!(c.id(), ID);
        assert_eq!(Ecosystem::kind(&c), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&c), &["php"]);
    }

    #[test]
    fn legacy_locator_tag_is_php() {
        assert_eq!(ExternalSourceLocator::ecosystem(&ComposerEcosystem), "php");
    }

    #[test]
    fn composer_json_parser_skips_platform_requirements() {
        let content = r#"{"require":{"php":">=8.0","ext-json":"*","laravel/framework":"^11.0"}}"#;
        let deps = parse_composer_json_deps(content);
        assert_eq!(deps, vec!["laravel/framework"]);
    }

    #[test]
    fn php_discovers_composer_deps() {
        let tmp = std::env::temp_dir().join("bw-test-composer-discover");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("composer.json"), r#"{"require":{"laravel/framework":"^11.0"}}"#).unwrap();
        let vendor = tmp.join("vendor").join("laravel").join("framework").join("src");
        std::fs::create_dir_all(&vendor).unwrap();
        std::fs::write(vendor.join("Application.php"), "<?php class Application {}\n").unwrap();

        let roots = discover_php_externals(&tmp);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].module_path, "laravel/framework");
        let files = walk_php_root(&roots[0]);
        assert_eq!(files.len(), 1);
        assert!(files[0].relative_path.contains("Application.php"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }

    // ----------------------------------------------------------------
    // R3 — user `use` scan + narrowed walk
    // ----------------------------------------------------------------

    #[test]
    fn php_use_extracts_fqn() {
        let mut out = std::collections::HashSet::new();
        extract_php_uses_from_source(
            "<?php\nuse Symfony\\Component\\HttpFoundation\\Request;\nuse App\\Service\\Foo as Bar;\nuse function Some\\helper;\nuse const Foo\\CONST_X;\n",
            &mut out,
        );
        assert!(out.contains("Symfony\\Component\\HttpFoundation\\Request"));
        assert!(out.contains("App\\Service\\Foo"));
        assert!(out.contains("Some\\helper"));
        assert!(out.contains("Foo\\CONST_X"));
    }

    #[test]
    fn php_group_use_explodes() {
        let mut out = std::collections::HashSet::new();
        extract_php_uses_from_source(
            "<?php\nuse Foo\\Bar\\{Baz, Qux as Q, Sub\\Thing};\n",
            &mut out,
        );
        assert!(out.contains("Foo\\Bar\\Baz"));
        assert!(out.contains("Foo\\Bar\\Qux"));
        assert!(out.contains("Foo\\Bar\\Sub\\Thing"));
    }

    #[test]
    fn php_fqn_suffix_uses_last_two_segments() {
        assert_eq!(
            php_fqn_to_path_suffix("Symfony\\Component\\HttpFoundation\\Request"),
            Some("HttpFoundation/Request.php".to_string())
        );
        assert_eq!(
            php_fqn_to_path_suffix("Foo\\Bar"),
            Some("Foo/Bar.php".to_string())
        );
        assert_eq!(php_fqn_to_path_suffix("Foo"), Some("Foo.php".to_string()));
        assert_eq!(php_fqn_to_path_suffix(""), None);
    }

    #[test]
    fn php_narrowed_walk_excludes_unreferenced_packages() {
        let tmp = std::env::temp_dir().join("bw-test-composer-r3-narrow");
        let _ = std::fs::remove_dir_all(&tmp);
        let dep_root = tmp.join("symfony").join("http-foundation");
        let src = dep_root.join("src");
        std::fs::create_dir_all(src.join("HttpFoundation")).unwrap();
        std::fs::create_dir_all(src.join("Unrelated")).unwrap();
        std::fs::write(
            src.join("HttpFoundation/Request.php"),
            "<?php class Request {}\n",
        ).unwrap();
        // Same-namespace sibling: included by virtue of Request matching. This
        // mirrors how PHP files reference same-namespace classes without a
        // `use` statement — walking the matched file but not its sibling
        // would leave those references unresolved.
        std::fs::write(
            src.join("HttpFoundation/Response.php"),
            "<?php class Response {}\n",
        ).unwrap();
        // Unrelated package (no matching FQN): must not be walked.
        std::fs::write(
            src.join("Unrelated/Thing.php"),
            "<?php class Thing {}\n",
        ).unwrap();

        let dep = ExternalDepRoot {
            module_path: "symfony/http-foundation".to_string(),
            version: "6.0".to_string(),
            root: dep_root.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: vec![
                "Symfony\\Component\\HttpFoundation\\Request".to_string(),
            ],
        };
        let files = walk_php_narrowed(&dep);
        let paths: std::collections::HashSet<_> =
            files.iter().map(|f| f.absolute_path.clone()).collect();
        assert!(paths.contains(&src.join("HttpFoundation/Request.php")));
        assert!(
            paths.contains(&src.join("HttpFoundation/Response.php")),
            "same-namespace sibling should be walked: {paths:?}"
        );
        assert!(
            !paths.contains(&src.join("Unrelated/Thing.php")),
            "unrelated package should not be walked: {paths:?}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn php_narrowed_walk_falls_back_when_no_imports() {
        let tmp = std::env::temp_dir().join("bw-test-composer-r3-fallback");
        let _ = std::fs::remove_dir_all(&tmp);
        let dep_root = tmp.join("foo").join("bar");
        let src = dep_root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("A.php"), "<?php class A {}\n").unwrap();

        let dep = ExternalDepRoot {
            module_path: "foo/bar".to_string(),
            version: "1.0".to_string(),
            root: dep_root.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };
        let files = walk_php_narrowed(&dep);
        assert_eq!(files.len(), 1);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
