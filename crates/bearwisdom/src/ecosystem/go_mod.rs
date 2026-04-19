// =============================================================================
// ecosystem/go_mod.rs — Go module ecosystem
//
// Phase 2 + 3: consolidates `indexer/externals/go.rs` +
// `indexer/manifest/go_mod.rs`. Go's module cache lives at
// `$GOMODCACHE/{escaped_module_path}@{version}`; indirect deps are walked
// only when a lightweight source scan confirms user code imports them.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader};
use crate::walker::WalkedFile;
use rayon::prelude::*;
use tree_sitter::{Node, Parser};

pub const ID: EcosystemId = EcosystemId::new("go-mod");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["go"];
const LEGACY_ECOSYSTEM_TAG: &str = "go";

pub struct GoModEcosystem;

impl Ecosystem for GoModEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("go"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_go_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_go_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        resolve_go_requested_packages(dep)
    }

    fn resolve_symbol(
        &self,
        dep: &ExternalDepRoot,
        _fqn: &str,
    ) -> Vec<WalkedFile> {
        resolve_go_requested_packages(dep)
    }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_go_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for GoModEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_go_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_go_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<GoModEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(GoModEcosystem)).clone()
}

// ===========================================================================
// Manifest reader (migrated from indexer/manifest/go_mod.rs)
// ===========================================================================

pub struct GoModManifest;

impl ManifestReader for GoModManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::GoMod }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let go_mod_path = find_go_mod(project_root)?;
        let content = std::fs::read_to_string(&go_mod_path).ok()?;
        let parsed = parse_go_mod(&content);
        let mut data = ManifestData::default();
        data.module_path = parsed.module_path;
        for path in parsed.require_paths {
            data.dependencies.insert(path);
        }
        Some(data)
    }
}

pub struct GoModData {
    pub module_path: Option<String>,
    pub require_paths: Vec<String>,
    pub require_deps: Vec<GoModDep>,
}

#[derive(Debug, Clone)]
pub struct GoModDep {
    pub path: String,
    pub version: String,
    pub indirect: bool,
}

pub fn find_go_mod(root: &Path) -> Option<PathBuf> {
    let candidate = root.join("go.mod");
    if candidate.is_file() { return Some(candidate) }
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let nested = entry.path().join("go.mod");
                if nested.is_file() { return Some(nested) }
            }
        }
    }
    None
}

pub fn parse_go_mod(content: &str) -> GoModData {
    let mut module_path: Option<String> = None;
    let mut require_paths = Vec::new();
    let mut require_deps = Vec::new();
    let mut in_require_block = false;

    fn parse_dep(fragment: &str) -> Option<GoModDep> {
        let without_comment = fragment.trim();
        let (main, comment) = match without_comment.find("//") {
            Some(idx) => (without_comment[..idx].trim(), &without_comment[idx..]),
            None => (without_comment, ""),
        };
        let mut tokens = main.split_whitespace();
        let path = tokens.next()?.to_string();
        let version = tokens.next()?.to_string();
        let indirect = comment.contains("indirect");
        Some(GoModDep { path, version, indirect })
    }

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") { continue }
        if let Some(rest) = trimmed.strip_prefix("module ") {
            let path = rest.split_whitespace().next().unwrap_or("").trim();
            if !path.is_empty() { module_path = Some(path.to_string()) }
            continue;
        }
        if trimmed == "require (" || trimmed.starts_with("require (") {
            in_require_block = true;
            continue;
        }
        if trimmed == ")" { in_require_block = false; continue }
        if let Some(rest) = trimmed.strip_prefix("require ") {
            let rest = rest.trim();
            if rest != "(" && !rest.is_empty() {
                if let Some(dep) = parse_dep(rest) {
                    require_paths.push(dep.path.clone());
                    require_deps.push(dep);
                }
            }
            continue;
        }
        if in_require_block && !trimmed.starts_with("//") {
            if let Some(dep) = parse_dep(trimmed) {
                require_paths.push(dep.path.clone());
                require_deps.push(dep);
            }
        }
    }

    GoModData { module_path, require_paths, require_deps }
}

// ===========================================================================
// Discovery — $GOMODCACHE / GOPATH / ~/go/pkg/mod
// ===========================================================================

pub fn discover_go_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let Some(go_mod_path) = find_go_mod(project_root) else { return Vec::new() };
    let Ok(content) = std::fs::read_to_string(&go_mod_path) else { return Vec::new() };
    let parsed = parse_go_mod(&content);

    let cache_root = match gomodcache_root() {
        Some(p) => p,
        None => {
            debug!("No GOMODCACHE / GOPATH detected; skipping Go externals");
            return Vec::new();
        }
    };

    let user_imports = collect_go_imports(project_root);

    let mut roots = Vec::new();
    for dep in &parsed.require_deps {
        if dep.indirect && !go_dep_is_imported(&dep.path, &user_imports) { continue }
        if let Some(root) = resolve_go_dep_path(&cache_root, dep) {
            let requested = collect_module_imports(&dep.path, &user_imports);
            roots.push(ExternalDepRoot {
                module_path: dep.path.clone(),
                version: dep.version.clone(),
                root,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: requested,
            });
        } else {
            debug!(
                "Go module cache miss for {}@{} — not found under {}",
                dep.path,
                dep.version,
                cache_root.display()
            );
        }
    }
    roots
}

/// Return the list of user import paths that live under `module_path`.
/// Includes the module root import itself (the main package) plus every
/// sub-package the project references. Paths are stored verbatim so
/// `resolve_go_requested_packages` can strip the module prefix when
/// mapping to on-disk subdirs.
fn collect_module_imports(
    module_path: &str,
    user_imports: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    let prefix = format!("{module_path}/");
    if user_imports.contains(module_path) { out.push(module_path.to_string()) }
    for imp in user_imports {
        if imp.starts_with(&prefix) { out.push(imp.clone()) }
    }
    out.sort();
    out.dedup();
    out
}

fn collect_go_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut imports: std::collections::HashSet<String> = std::collections::HashSet::new();
    scan_go_imports_recursive(project_root, &mut imports, 0);
    imports
}

fn scan_go_imports_recursive(
    dir: &Path,
    out: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 10 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(
                        name,
                        ".git" | "vendor" | "node_modules" | "target"
                            | "build" | "dist" | "testdata"
                    ) { continue }
                }
                scan_go_imports_recursive(&path, out, depth + 1);
            } else if ft.is_file() {
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
                if !name.ends_with(".go") || name.ends_with("_test.go") { continue }
                let Ok(content) = std::fs::read_to_string(&path) else { continue };
                extract_imports_from_go_source(&content, out);
            }
        }
    }
}

fn extract_imports_from_go_source(content: &str, out: &mut std::collections::HashSet<String>) {
    enum Mode { Top, InBlock }
    let mut mode = Mode::Top;
    for line in content.lines() {
        let trimmed = line.trim();
        match mode {
            Mode::Top => {
                if trimmed.starts_with("import (") { mode = Mode::InBlock; continue }
                if let Some(rest) = trimmed.strip_prefix("import ") {
                    let rest = rest.trim_start_matches('_').trim();
                    let quoted = rest
                        .rsplit_once('"')
                        .map(|(head, _)| head)
                        .and_then(|head| head.rsplit_once('"').map(|(_, s)| s));
                    if let Some(path) = quoted {
                        if !path.is_empty() { out.insert(path.to_string()); }
                    }
                }
            }
            Mode::InBlock => {
                if trimmed == ")" { mode = Mode::Top; continue }
                let bytes = trimmed.as_bytes();
                let first = bytes.iter().position(|&b| b == b'"');
                let Some(start) = first else { continue };
                let after = &trimmed[start + 1..];
                let Some(end_rel) = after.find('"') else { continue };
                let path = &after[..end_rel];
                if !path.is_empty() { out.insert(path.to_string()); }
            }
        }
    }
}

fn go_dep_is_imported(
    dep_path: &str,
    user_imports: &std::collections::HashSet<String>,
) -> bool {
    if user_imports.contains(dep_path) { return true }
    let prefix = format!("{dep_path}/");
    user_imports.iter().any(|imp| imp.starts_with(&prefix))
}

pub fn gomodcache_root() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("GOMODCACHE") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p) }
    }
    if let Some(gopath) = std::env::var_os("GOPATH") {
        let first = PathBuf::from(gopath)
            .to_string_lossy()
            .split(|c| c == ':' || c == ';')
            .next()
            .map(PathBuf::from);
        if let Some(p) = first {
            let candidate = p.join("pkg").join("mod");
            if candidate.is_dir() { return Some(candidate) }
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join("go").join("pkg").join("mod");
    if candidate.is_dir() { Some(candidate) } else { None }
}

fn resolve_go_dep_path(cache_root: &Path, dep: &GoModDep) -> Option<PathBuf> {
    let escaped = escape_module_path(&dep.path);
    let dirname = format!("{}@{}", escaped, dep.version);
    let candidate = cache_root.join(dirname.replace('/', std::path::MAIN_SEPARATOR_STR));
    if candidate.is_dir() { return Some(candidate) }
    let mut segments: Vec<&str> = escaped.split('/').collect();
    let last = segments.pop()?;
    let mut path = cache_root.to_path_buf();
    for seg in segments { path.push(seg); }
    path.push(format!("{last}@{}", dep.version));
    if path.is_dir() { Some(path) } else { None }
}

fn escape_module_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 4);
    for ch in path.chars() {
        if ch.is_ascii_uppercase() {
            out.push('!');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Reachability: narrow to user-requested sub-packages + transitive within-module
// ---------------------------------------------------------------------------
//
// Go is unlike npm/pypi/cargo because a single module exposes many
// independent sub-packages (flat-directory `package X { ... }` files, no
// explicit re-export mechanism). A typical user project imports only a
// handful of sub-packages from each module. The eager walk_root strategy
// indexes every package in the module even when the user's surface is
// narrow — wasteful on monster modules like `k8s.io/api` or
// `google.golang.org/protobuf`.
//
// Reachability for Go: walk only the sub-directories corresponding to the
// user's import paths (stored on the dep root at discovery time) plus any
// within-module imports those packages pull in transitively. Each Go
// package is flat (no recursion into subdirs — subdirs are separate
// packages with their own imports), so the walk per package is a single
// `read_dir` scan.

const GO_SUBPKG_MAX_DEPTH: u32 = 3;

fn resolve_go_requested_packages(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() {
        // Discovery didn't populate demand data; fall back to the eager walk
        // so behavior matches pre-R3 when user_imports scanning was absent.
        return walk_go_root(dep);
    }

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for import_path in &dep.requested_imports {
        let Some(sub) = import_path.strip_prefix(&dep.module_path[..]) else {
            continue;
        };
        let sub = sub.trim_start_matches('/');
        let pkg_dir = if sub.is_empty() {
            dep.root.clone()
        } else {
            dep.root.join(sub.replace('/', std::path::MAIN_SEPARATOR_STR.to_string().as_str()))
        };
        expand_go_package_into(dep, &pkg_dir, &mut out, &mut seen, 0);
    }
    out
}

fn expand_go_package_into(
    dep: &ExternalDepRoot,
    pkg_dir: &Path,
    out: &mut Vec<WalkedFile>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: u32,
) {
    if !pkg_dir.is_dir() { return }
    if !seen.insert(pkg_dir.to_path_buf()) { return }

    let Ok(entries) = std::fs::read_dir(pkg_dir) else { return };
    let mut package_sources: Vec<std::path::PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() { continue }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !name.ends_with(".go") { continue }
        if name.ends_with("_test.go") { continue }
        if !super::go_platform::file_matches_host(name) { continue }
        let rel_sub = match path.strip_prefix(&dep.root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        out.push(WalkedFile {
            relative_path: format!("ext:{}@{}/{}", dep.module_path, dep.version, rel_sub),
            absolute_path: path.clone(),
            language: "go",
        });
        package_sources.push(path);
    }

    if depth >= GO_SUBPKG_MAX_DEPTH { return }

    // Scan this package's source for within-module imports and pull those
    // sub-packages in too. Without this, a user-imported package that
    // re-exports types from a sibling sub-package loses the type
    // definitions it needs.
    let module_prefix = format!("{}/", dep.module_path);
    for path in &package_sources {
        let Ok(content) = std::fs::read_to_string(path) else { continue };
        let mut imports: std::collections::HashSet<String> = std::collections::HashSet::new();
        extract_imports_from_go_source(&content, &mut imports);
        for imp in imports {
            if !imp.starts_with(&module_prefix) { continue }
            let Some(sub) = imp.strip_prefix(&dep.module_path[..]) else { continue };
            let sub = sub.trim_start_matches('/');
            if sub.is_empty() { continue }
            let sub_dir = dep.root.join(sub.replace('/', std::path::MAIN_SEPARATOR_STR.to_string().as_str()));
            expand_go_package_into(dep, &sub_dir, out, seen, depth + 1);
        }
    }
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_go_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "vendor" | "testdata" | ".git" | "_examples") { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".go") { continue }
            if name.ends_with("_test.go") { continue }
            if !super::go_platform::file_matches_host(name) { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:{}@{}/{}", dep.module_path, dep.version, rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "go",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline scaffolding)
// ---------------------------------------------------------------------------
//
// Builds a cheap `(module_path, symbol_name) → file` map over every reached
// Go dep root by tree-sitter parsing each .go file and reading ONLY the
// top-level declarations (functions, methods, types, vars, consts). Function
// bodies are never walked — their inner nodes aren't inspected, so the
// allocation profile is O(#top_level_decls) rather than O(#ast_nodes). That
// cost is the one-time entry fee the demand-driven Stage 2 pipeline pays so
// it can then pull just the files its demand set actually needs.
//
// File scope matches `resolve_go_requested_packages` — only sub-packages the
// user imports, plus within-module transitives up to GO_SUBPKG_MAX_DEPTH.
// Platform-mismatched files are dropped via `go_platform::file_matches_host`.

fn build_go_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    // Collect every walked file + its owning module path so each parallel
    // scanner task is self-contained.
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in resolve_go_requested_packages(dep) {
            work.push((dep.module_path.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }

    // Parallel header-only scan. Each task returns (module, name, file)
    // tuples; we merge into one index at the end so the hot map isn't
    // under a lock during the scan.
    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_go_header(&src)
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

/// Header-only tree-sitter scan of a Go source file. Returns the list of
/// top-level declaration names the file exports (or defines locally — the
/// caller filters by visibility if needed). Methods are keyed as
/// `ReceiverType.MethodName`; struct/interface/alias types are keyed by
/// their type name; top-level vars and consts are keyed by their identifier.
///
/// Bodies of functions and methods are *not* walked — we only inspect the
/// direct children of `source_file` and the immediate children of the
/// type/var/const declarations. No `block` is descended.
fn scan_go_header(source: &str) -> Vec<String> {
    let language = tree_sitter_go::LANGUAGE.into();
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
        match child.kind() {
            "function_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Ok(name) = name_node.utf8_text(bytes) {
                        out.push(name.to_string());
                    }
                }
            }
            "method_declaration" => {
                let (recv, name) = method_decl_names(&child, source);
                match (recv, name) {
                    (Some(recv), Some(name)) => out.push(format!("{recv}.{name}")),
                    // Receiver type couldn't be parsed — still record the
                    // bare method name so unresolved lookups have something
                    // to match against.
                    (None, Some(name)) => out.push(name),
                    _ => {}
                }
            }
            "type_declaration" => {
                let mut sub_cursor = child.walk();
                for spec in child.children(&mut sub_cursor) {
                    if matches!(spec.kind(), "type_spec" | "type_alias") {
                        if let Some(name_node) = spec.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                out.push(name.to_string());
                            }
                        }
                    }
                }
            }
            "var_declaration" | "const_declaration" => {
                // Grouped declarations `var ( ... )` wrap their specs in a
                // `var_spec_list` / `const_spec_list` intermediate; single
                // specs are direct children. Handle both.
                collect_var_const_names(&child, bytes, &mut out);
            }
            _ => {}
        }
    }

    out
}

/// Walk a `var_declaration` / `const_declaration` node and append every
/// declared name onto `out`. Handles the grouped form `var ( a = 1; b = 2 )`
/// where tree-sitter-go wraps the specs in a `var_spec_list` intermediate.
fn collect_var_const_names(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "var_spec" | "const_spec" => collect_spec_names(&child, bytes, out),
            "var_spec_list" | "const_spec_list" => {
                // Recurse one level to reach the individual specs.
                let mut inner = child.walk();
                for spec in child.children(&mut inner) {
                    if matches!(spec.kind(), "var_spec" | "const_spec") {
                        collect_spec_names(&spec, bytes, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Pull identifier names out of a single `var_spec` / `const_spec`. Stops
/// at the first non-identifier named child (the declared type) or at `=`
/// (the start of the RHS expression list).
fn collect_spec_names(spec: &Node, bytes: &[u8], out: &mut Vec<String>) {
    let mut cursor = spec.walk();
    let mut past_names = false;
    for cc in spec.children(&mut cursor) {
        if !cc.is_named() {
            if cc.utf8_text(bytes) == Ok("=") {
                past_names = true;
            }
            continue;
        }
        if past_names { break }
        if cc.kind() == "identifier" {
            if let Ok(name) = cc.utf8_text(bytes) {
                out.push(name.to_string());
            }
        } else {
            past_names = true;
        }
    }
}

/// Pull `(receiver_type_name, method_name)` out of a `method_declaration`
/// node without walking its body. Handles `*T`, `T`, and `T[K]` receivers.
fn method_decl_names(node: &Node, source: &str) -> (Option<String>, Option<String>) {
    let mut receiver: Option<String> = None;
    let mut method: Option<String> = None;
    let mut seen_receiver_list = false;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() { continue }
        match child.kind() {
            "parameter_list" if !seen_receiver_list => {
                seen_receiver_list = true;
                receiver = extract_receiver_type(&child, source);
            }
            "field_identifier" if method.is_none() => {
                if let Ok(s) = child.utf8_text(source.as_bytes()) {
                    method = Some(s.to_string());
                }
            }
            _ => {}
        }
    }
    (receiver, method)
}

fn extract_receiver_type(param_list: &Node, source: &str) -> Option<String> {
    let bytes = source.as_bytes();
    let mut cursor = param_list.walk();
    for child in param_list.children(&mut cursor) {
        if child.kind() != "parameter_declaration" { continue }
        let mut ccursor = child.walk();
        for cc in child.children(&mut ccursor) {
            if !cc.is_named() { continue }
            match cc.kind() {
                "type_identifier" => {
                    return cc.utf8_text(bytes).ok().map(String::from);
                }
                "pointer_type" => {
                    // *T — find the inner type_identifier / generic_type.
                    let mut inner = cc.walk();
                    for t in cc.children(&mut inner) {
                        if t.kind() == "type_identifier" {
                            return t.utf8_text(bytes).ok().map(String::from);
                        }
                        if t.kind() == "generic_type" {
                            if let Some(inner_name) = t.child_by_field_name("type") {
                                return inner_name.utf8_text(bytes).ok().map(String::from);
                            }
                        }
                    }
                }
                "generic_type" => {
                    // T[K] — unwrap to T.
                    if let Some(inner_name) = cc.child_by_field_name("type") {
                        return inner_name.utf8_text(bytes).ok().map(String::from);
                    }
                }
                _ => {}
            }
        }
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
        let g = GoModEcosystem;
        assert_eq!(g.id(), ID);
        assert_eq!(Ecosystem::kind(&g), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&g), &["go"]);
    }

    #[test]
    fn legacy_locator_tag_is_go() {
        assert_eq!(ExternalSourceLocator::ecosystem(&GoModEcosystem), "go");
    }

    #[test]
    fn escape_preserves_lowercase_paths() {
        assert_eq!(
            escape_module_path("github.com/gin-gonic/gin"),
            "github.com/gin-gonic/gin"
        );
    }

    #[test]
    fn escape_handles_uppercase_segments() {
        assert_eq!(
            escape_module_path("github.com/Microsoft/go-winio"),
            "github.com/!microsoft/go-winio"
        );
        assert_eq!(
            escape_module_path("github.com/AlecAivazis/survey"),
            "github.com/!alec!aivazis/survey"
        );
    }

    #[test]
    fn discover_returns_empty_without_go_mod() {
        let tmp = std::env::temp_dir().join("bw-test-gomod-empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let result = discover_go_externals(&tmp);
        assert!(result.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parse_go_mod_basic() {
        let content = r#"module foo.example/bar

go 1.21

require (
    github.com/gin-gonic/gin v1.9.1
    github.com/stretchr/testify v1.9.0 // indirect
)

require github.com/other/pkg v1.0.0
"#;
        let data = parse_go_mod(content);
        assert_eq!(data.module_path.as_deref(), Some("foo.example/bar"));
        assert_eq!(data.require_deps.len(), 3);
        assert!(data.require_deps.iter().any(|d| d.path == "github.com/gin-gonic/gin" && !d.indirect));
        assert!(data.require_deps.iter().any(|d| d.path == "github.com/stretchr/testify" && d.indirect));
        assert!(data.require_deps.iter().any(|d| d.path == "github.com/other/pkg"));
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }

    // -----------------------------------------------------------------
    // Header-only scanner — tree-sitter parse without body descent
    // -----------------------------------------------------------------

    #[test]
    fn scan_captures_top_level_function() {
        let src = r#"
package sqlite

func Open(path string) (*DB, error) {
    // body we never walk
    return nil, nil
}
"#;
        let names = scan_go_header(src);
        assert!(names.contains(&"Open".to_string()), "names: {names:?}");
    }

    #[test]
    fn scan_captures_method_with_pointer_receiver() {
        let src = r#"
package sqlite

type DB struct{}

func (d *DB) Query(q string) (*Rows, error) {
    return nil, nil
}
"#;
        let names = scan_go_header(src);
        assert!(names.contains(&"DB".to_string()), "types missing: {names:?}");
        assert!(names.contains(&"DB.Query".to_string()), "method missing: {names:?}");
    }

    #[test]
    fn scan_captures_method_with_value_receiver() {
        let src = r#"
package sqlite

type Query struct{}

func (q Query) String() string { return "" }
"#;
        let names = scan_go_header(src);
        assert!(names.contains(&"Query.String".to_string()), "{names:?}");
    }

    #[test]
    fn scan_captures_type_declarations() {
        let src = r#"
package foo

type Client struct { addr string }
type Handler interface { Handle() }
type Name = string
"#;
        let names = scan_go_header(src);
        assert!(names.contains(&"Client".to_string()), "{names:?}");
        assert!(names.contains(&"Handler".to_string()), "{names:?}");
        assert!(names.contains(&"Name".to_string()), "{names:?}");
    }

    #[test]
    fn scan_captures_top_level_vars_and_consts() {
        let src = r#"
package foo

var DefaultTimeout = 30
const MaxRetries = 3

var (
    LogLevel = "info"
    Verbose  bool
)
"#;
        let names = scan_go_header(src);
        for expected in ["DefaultTimeout", "MaxRetries", "LogLevel", "Verbose"] {
            assert!(
                names.contains(&expected.to_string()),
                "missing {expected}: {names:?}"
            );
        }
    }

    #[test]
    fn scan_ignores_function_body_contents() {
        // Identifiers inside function bodies must not leak into the top-level
        // name set — the scanner is *header-only*.
        let src = r#"
package foo

func Outer() {
    var shouldNotAppear = 1
    type AlsoHidden struct{}
    _ = shouldNotAppear
}
"#;
        let names = scan_go_header(src);
        assert_eq!(names, vec!["Outer".to_string()]);
    }

    #[test]
    fn scan_handles_generic_method_receiver() {
        // `func (c *Cache[K, V]) Get(k K) V` — receiver type unwraps to `Cache`.
        let src = r#"
package foo

type Cache[K comparable, V any] struct{}

func (c *Cache[K, V]) Get(k K) V { var zero V; return zero }
"#;
        let names = scan_go_header(src);
        assert!(names.contains(&"Cache".to_string()), "type: {names:?}");
        assert!(names.contains(&"Cache.Get".to_string()), "method: {names:?}");
    }

    #[test]
    fn scan_returns_empty_on_unparseable_source() {
        // Tree-sitter returns an error tree rather than None, but we should
        // still surface whatever valid top-level decls it finds.
        let names = scan_go_header("not valid go");
        assert!(names.is_empty());
    }

    #[test]
    fn build_index_returns_empty_for_no_deps() {
        let idx = build_go_symbol_index(&[]);
        assert!(idx.is_empty());
    }

    #[test]
    fn build_index_populates_from_on_disk_files() {
        use std::io::Write;
        let tmp = std::env::temp_dir().join("bw-test-gomod-symindex");
        let _ = std::fs::remove_dir_all(&tmp);
        let pkg_dir = tmp.join("lib");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        let sqlite_go = pkg_dir.join("sqlite.go");
        let mut f = std::fs::File::create(&sqlite_go).unwrap();
        writeln!(
            f,
            "package sqlite\n\ntype DB struct {{}}\nfunc Open() *DB {{ return nil }}\nfunc (d *DB) Query() error {{ return nil }}"
        ).unwrap();

        let dep = ExternalDepRoot {
            module_path: "modernc.org/sqlite".to_string(),
            version: "v0".to_string(),
            root: tmp.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: vec!["modernc.org/sqlite/lib".to_string()],
        };
        let idx = build_go_symbol_index(std::slice::from_ref(&dep));

        assert_eq!(
            idx.locate("modernc.org/sqlite", "Open"),
            Some(sqlite_go.as_path())
        );
        assert_eq!(
            idx.locate("modernc.org/sqlite", "DB"),
            Some(sqlite_go.as_path())
        );
        assert_eq!(
            idx.locate("modernc.org/sqlite", "DB.Query"),
            Some(sqlite_go.as_path())
        );
        assert_eq!(idx.locate("modernc.org/sqlite", "Nonexistent"), None);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------
    // R3 — user-import-narrowed sub-package walking
    // -----------------------------------------------------------------

    fn mkdep(root: PathBuf, module: &str, requested: Vec<String>) -> ExternalDepRoot {
        ExternalDepRoot {
            module_path: module.to_string(),
            version: "v1.0.0".to_string(),
            root,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: requested,
        }
    }

    #[test]
    fn collect_module_imports_returns_matching_paths() {
        let mut set = std::collections::HashSet::new();
        set.insert("github.com/gin-gonic/gin".to_string());
        set.insert("github.com/gin-gonic/gin/binding".to_string());
        set.insert("github.com/gin-gonic/gin/render".to_string());
        set.insert("github.com/other/pkg".to_string());
        set.insert("github.com/gin-gonic/gink".to_string()); // prefix collision — must NOT match
        let got = collect_module_imports("github.com/gin-gonic/gin", &set);
        assert_eq!(
            got,
            vec![
                "github.com/gin-gonic/gin".to_string(),
                "github.com/gin-gonic/gin/binding".to_string(),
                "github.com/gin-gonic/gin/render".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_walks_only_requested_sub_packages() {
        let tmp = std::env::temp_dir().join("bw-test-go-r3-narrow");
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("gin@v1.0.0");
        let binding = root.join("binding");
        let internal = root.join("internal");
        std::fs::create_dir_all(&binding).unwrap();
        std::fs::create_dir_all(&internal).unwrap();
        std::fs::write(root.join("gin.go"), "package gin\n").unwrap();
        std::fs::write(binding.join("binding.go"), "package binding\n").unwrap();
        std::fs::write(internal.join("internal.go"), "package internal\n").unwrap();

        let dep = mkdep(
            root.clone(),
            "github.com/gin-gonic/gin",
            vec!["github.com/gin-gonic/gin/binding".to_string()],
        );
        let files = resolve_go_requested_packages(&dep);
        let paths: std::collections::HashSet<_> =
            files.iter().map(|f| f.absolute_path.clone()).collect();
        assert!(
            paths.contains(&binding.join("binding.go")),
            "expected binding.go to be walked, got: {paths:?}"
        );
        assert!(
            !paths.contains(&root.join("gin.go")),
            "root package should NOT be walked when not requested: {paths:?}"
        );
        assert!(
            !paths.contains(&internal.join("internal.go")),
            "unrequested sibling should NOT be walked: {paths:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_follows_within_module_transitive_imports() {
        let tmp = std::env::temp_dir().join("bw-test-go-r3-transitive");
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("myMod@v1.0.0");
        let a = root.join("a");
        let b = root.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        // a/a.go imports b, so walking a should pull in b.
        std::fs::write(
            a.join("a.go"),
            "package a\nimport \"my.example/myMod/b\"\nvar _ = b.X\n",
        ).unwrap();
        std::fs::write(b.join("b.go"), "package b\nvar X = 1\n").unwrap();

        let dep = mkdep(
            root.clone(),
            "my.example/myMod",
            vec!["my.example/myMod/a".to_string()],
        );
        let files = resolve_go_requested_packages(&dep);
        let paths: std::collections::HashSet<_> =
            files.iter().map(|f| f.absolute_path.clone()).collect();
        assert!(paths.contains(&a.join("a.go")));
        assert!(paths.contains(&b.join("b.go")),
            "transitive within-module import should be followed: {paths:?}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_falls_back_to_walk_root_when_no_requested_imports() {
        let tmp = std::env::temp_dir().join("bw-test-go-r3-fallback");
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("mod@v1.0.0");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.go"), "package mod\n").unwrap();

        let dep = mkdep(root.clone(), "my.example/mod", Vec::new());
        let files = resolve_go_requested_packages(&dep);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].absolute_path, root.join("a.go"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
