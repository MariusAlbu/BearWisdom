// =============================================================================
// ecosystem/opam.rs — opam ecosystem (OCaml)
//
// Phase 2 + 3: consolidates `indexer/externals/ocaml.rs` +
// `indexer/manifest/opam.rs`.
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
use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("opam");
const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["ocaml"];
const LEGACY_ECOSYSTEM_TAG: &str = "ocaml";

pub struct OpamEcosystem;

impl Ecosystem for OpamEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        &[("dune-project", "ocaml")]
    }

    fn workspace_package_extensions(&self) -> &'static [(&'static str, &'static str)] {
        &[(".opam", "ocaml")]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["_build", "_opam"]
    }

    fn activation(&self) -> EcosystemActivation {
        // Project deps via an `*.opam` file (or `dune-project`). A bare
        // directory of `.ml` files with no manifest can't be resolved
        // against external opam coordinates, so dropping the
        // LanguagePresent shotgun is correct per the trait doc.
        EcosystemActivation::ManifestMatch
    }
    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_ocaml_externals(ctx.project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk_ocaml_root(dep) }
    fn supports_reachability(&self) -> bool { true }
    fn resolve_import(
        &self, dep: &ExternalDepRoot, _p: &str, _s: &[&str],
    ) -> Vec<WalkedFile> { walk_ocaml_narrowed(dep) }
    fn resolve_symbol(
        &self, dep: &ExternalDepRoot, _f: &str,
    ) -> Vec<WalkedFile> { walk_ocaml_narrowed(dep) }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_ocaml_symbol_index(dep_roots)
    }

    /// Stdlib substrate (List, String, Array, Printf, Buffer, Bytes, ...) is
    /// auto-opened in every OCaml compilation unit. Bare uses like
    /// `List.fold_left` and `String.length` don't appear as imports for the
    /// demand BFS to chase, so the substrate files would never get pulled.
    /// Pre-pulling each `ocaml` and `stdlib-shims` root makes their symbols
    /// reachable for any project. Third-party deps stay demand-driven.
    fn demand_pre_pull(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> Vec<WalkedFile> {
        dep_roots
            .iter()
            .filter(|d| matches!(d.module_path.as_str(), "ocaml" | "stdlib-shims"))
            .flat_map(walk_ocaml_root)
            .collect()
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for OpamEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_ocaml_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk_ocaml_root(dep) }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<OpamEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(OpamEcosystem)).clone()
}

// ===========================================================================
// Manifest reader
// ===========================================================================

pub struct OpamManifest;

impl ManifestReader for OpamManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Opam }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let Ok(entries) = std::fs::read_dir(project_root) else { return None };
        let opam_file = entries.flatten().find(|e| {
            e.path().extension().and_then(|x| x.to_str()) == Some("opam")
        })?;
        let content = std::fs::read_to_string(opam_file.path()).ok()?;
        let mut data = ManifestData::default();
        for name in parse_opam_depends(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

pub fn parse_opam_depends(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let Some(start) = content.find("depends:") else { return deps };
    let rest = &content[start + "depends:".len()..];
    let Some(bracket_start) = rest.find('[') else { return deps };
    let rest = &rest[bracket_start + 1..];
    let Some(bracket_end) = rest.find(']') else { return deps };
    let block = &rest[..bracket_end];

    for line in block.lines() {
        let trimmed = line.trim().trim_start_matches('"');
        if trimmed.is_empty() { continue }
        let name = trimmed.split(|c: char| c == '"' || c == ' ' || c == '{')
            .next().unwrap_or("").trim();
        if !name.is_empty()
            && name != "ocaml"
            && !name.starts_with("conf-")
            && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            if !deps.contains(&name.to_string()) { deps.push(name.to_string()) }
        }
    }
    deps
}

// ===========================================================================
// Discovery + walk
// ===========================================================================

pub fn discover_ocaml_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let Ok(entries) = std::fs::read_dir(project_root) else { return Vec::new() };
    let opam_file = entries.flatten().find(|e| {
        e.path().extension().and_then(|x| x.to_str()) == Some("opam")
    });
    let Some(opam_entry) = opam_file else { return Vec::new() };
    let Ok(content) = std::fs::read_to_string(opam_entry.path()) else { return Vec::new() };
    let declared = parse_opam_depends(&content);
    if declared.is_empty() { return Vec::new() }

    let lib_dirs = ocaml_lib_dirs(project_root);
    let user_modules: Vec<String> = collect_ocaml_user_modules(project_root)
        .into_iter()
        .collect();

    let mut roots = Vec::new();
    for dep in &declared {
        for lib in &lib_dirs {
            let pkg_dir = lib.join(dep);
            if pkg_dir.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version: String::new(),
                    root: pkg_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: user_modules.clone(),
                });
                break;
            }
        }
    }
    // OCaml Stdlib + shims are language substrate, not project-declared deps.
    // Every project uses Printf, List, String, Array, Buffer, Bytes, etc.
    for substrate in ["ocaml", "stdlib-shims"] {
        for lib in &lib_dirs {
            let pkg_dir = lib.join(substrate);
            if pkg_dir.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: substrate.to_string(),
                    version: String::new(),
                    root: pkg_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: user_modules.clone(),
                });
                break;
            }
        }
    }
    debug!("OCaml: {} external package roots", roots.len());
    roots
}

// R3 — `open Foo`, `Foo.Bar.func`, `module M = Foo.Bar` scanner. OCaml
// module names are CamelCase; the corresponding source file is lowercase
// (`foo.ml` for module `Foo`).

fn collect_ocaml_user_modules(project_root: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    scan_ocaml_modules(project_root, &mut out, 0);
    out
}

fn scan_ocaml_modules(dir: &Path, out: &mut std::collections::HashSet<String>, depth: usize) {
    if depth > 12 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, ".git" | "_build" | "_opam" | "test" | "tests" | "bench")
                    || name.starts_with('.') { continue }
            }
            scan_ocaml_modules(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(name.ends_with(".ml") || name.ends_with(".mli")) { continue }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            extract_ocaml_modules(&content, out);
        }
    }
}

fn extract_ocaml_modules(content: &str, out: &mut std::collections::HashSet<String>) {
    for raw in content.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("open ") {
            let head = rest
                .split(|c: char| c == ' ' || c == '\t' || c == ';')
                .next()
                .unwrap_or("")
                .trim();
            if let Some(top) = head.split('.').next() {
                if !top.is_empty() && top.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
                    out.insert(top.to_string());
                }
            }
            continue;
        }
        // Bare `Foo.Bar.x` style references — pull the leading capitalized run.
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_uppercase() {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let tok = &line[start..i];
                // Only keep if followed by `.` — that's the marker of "use as module".
                if i < bytes.len() && bytes[i] == b'.' {
                    out.insert(tok.to_string());
                }
            } else {
                i += 1;
            }
        }
    }
}

fn ocaml_module_to_path_tail(module: &str) -> Option<String> {
    let cleaned = module.trim();
    if cleaned.is_empty() { return None }
    Some(format!("{}.ml", cleaned.to_ascii_lowercase()))
}

fn walk_ocaml_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() { return walk_ocaml_root(dep); }
    let tails: std::collections::HashSet<String> = dep
        .requested_imports
        .iter()
        .filter_map(|m| ocaml_module_to_path_tail(m))
        .collect();
    if tails.is_empty() { return walk_ocaml_root(dep); }

    let mut out = Vec::new();
    walk_ocaml_narrowed_dir(&dep.root, &dep.root, dep, &tails, &mut out, 0);
    out
}

fn walk_ocaml_narrowed_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    tails: &std::collections::HashSet<String>,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut subdirs: Vec<PathBuf> = Vec::new();
    let mut dir_files: Vec<(PathBuf, String)> = Vec::new();
    let mut any_match = false;

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "bench") || name.starts_with('.') { continue }
            }
            subdirs.push(path);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(name.ends_with(".ml") || name.ends_with(".mli")) { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            // Match using `.ml` as the canonical tail; .mli pairs the .ml so
            // include both via basename comparison.
            let basename = rel_sub.rsplit('/').next().unwrap_or(&rel_sub);
            let stem = basename.trim_end_matches(".mli").trim_end_matches(".ml");
            if tails.iter().any(|t| t.trim_end_matches(".ml") == stem) { any_match = true; }
            dir_files.push((path, rel_sub));
        }
    }

    if any_match {
        for (path, rel_sub) in dir_files {
            out.push(WalkedFile {
                relative_path: format!("ext:ocaml:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "ocaml",
            });
        }
    }
    for sub in subdirs {
        walk_ocaml_narrowed_dir(&sub, root, dep, tails, out, depth + 1);
    }
}

fn ocaml_lib_dirs(project_root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let local_opam = project_root.join("_opam").join("lib");
    if local_opam.is_dir() { dirs.push(local_opam) }
    if let Ok(switch) = std::env::var("OPAM_SWITCH_PREFIX") {
        let lib = PathBuf::from(switch).join("lib");
        if lib.is_dir() { dirs.push(lib) }
    }
    let mut opam_roots: Vec<PathBuf> = Vec::new();
    if let Ok(root) = std::env::var("OPAMROOT") {
        opam_roots.push(PathBuf::from(root));
    }
    if let Some(home) = dirs::home_dir() {
        opam_roots.push(home.join(".opam"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        opam_roots.push(PathBuf::from(local).join("opam"));
    }
    for opam in opam_roots {
        let Ok(entries) = std::fs::read_dir(&opam) else { continue };
        for e in entries.flatten() {
            let lib = e.path().join("lib");
            if lib.is_dir() { dirs.push(lib) }
        }
    }
    dirs
}

fn walk_ocaml_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "bench") || name.starts_with('.') { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(name.ends_with(".ml") || name.ends_with(".mli")) { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:ocaml:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "ocaml",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------

fn build_ocaml_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_ocaml_root(dep) {
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
            scan_ocaml_header(&src)
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

/// Header-only tree-sitter scan of an OCaml source file. Records top-level
/// `let`, `module`, `type`, `val`, `external`, `exception` bindings.
fn scan_ocaml_header(source: &str) -> Vec<String> {
    let language = tree_sitter_ocaml::LANGUAGE_OCAML.into();
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
        collect_ocaml_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_ocaml_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "value_definition" | "let_binding" | "module_definition"
        | "module_binding" | "type_definition" | "type_binding"
        | "external" | "exception_definition" | "class_definition"
        | "class_binding" => {
            // Try common field names, then fall back to first identifier child.
            let name_node = node
                .child_by_field_name("name")
                .or_else(|| node.child_by_field_name("pattern"))
                .or_else(|| first_identifier_child(node));
            if let Some(name_node) = name_node {
                if let Ok(t) = name_node.utf8_text(bytes) {
                    out.push(t.to_string());
                }
            }
        }
        _ => {}
    }
}

fn first_identifier_child<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "value_name"
                | "value_pattern"
                | "module_name"
                | "type_constructor"
                | "constructor_name"
                | "lowercase_identifier"
                | "capitalized_identifier"
        ) {
            return Some(child);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        assert_eq!(OpamEcosystem.id(), ID);
        assert_eq!(Ecosystem::languages(&OpamEcosystem), &["ocaml"]);
    }

    #[test]
    fn parse_opam_deps() {
        let content = r#"
depends: [
  "dune" {>= "2.8.0"}
  "ocaml" {>= "4.08.1"}
  "conf-libpcre"
  "cohttp-lwt-unix"
  "core"
  "lwt"
]
"#;
        let deps = parse_opam_depends(content);
        assert!(deps.contains(&"cohttp-lwt-unix".to_string()));
        assert!(deps.contains(&"core".to_string()));
        assert!(deps.contains(&"lwt".to_string()));
        assert!(!deps.contains(&"ocaml".to_string()));
        assert!(!deps.contains(&"conf-libpcre".to_string()));
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }

    #[test]
    fn ocaml_extracts_open_and_dotted() {
        let mut out = std::collections::HashSet::new();
        extract_ocaml_modules(
            "open Core\nopen Lwt.Infix\nlet x = Cohttp_lwt_unix.Client.get url\n",
            &mut out,
        );
        assert!(out.contains("Core"));
        assert!(out.contains("Lwt"));
        assert!(out.contains("Cohttp_lwt_unix"));
    }

    #[test]
    fn ocaml_module_path_tail_is_lowercase_ml() {
        assert_eq!(ocaml_module_to_path_tail("Core"), Some("core.ml".to_string()));
        assert_eq!(ocaml_module_to_path_tail("Cohttp_lwt_unix"), Some("cohttp_lwt_unix.ml".to_string()));
    }
}
