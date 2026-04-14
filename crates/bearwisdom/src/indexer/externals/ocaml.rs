// OCaml / opam externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// OCaml opam → `discover_ocaml_externals` + `walk_ocaml_external_root`.
///
/// opam packages live in `_opam/lib/<pkg>/` (local switch) or
/// `~/.opam/<switch>/lib/<pkg>/` (global switch).
/// Declared deps come from `*.opam` files `depends:` field.
/// Walk: `*.ml` and `*.mli` files.
pub struct OcamlExternalsLocator;

impl ExternalSourceLocator for OcamlExternalsLocator {
    fn ecosystem(&self) -> &'static str { "ocaml" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_ocaml_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ocaml_external_root(dep)
    }
}

pub fn discover_ocaml_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::opam::parse_opam_depends;

    let Ok(entries) = std::fs::read_dir(project_root) else { return Vec::new(); };
    let opam_file = entries.flatten().find(|e| {
        e.path().extension().and_then(|x| x.to_str()) == Some("opam")
    });
    let Some(opam_entry) = opam_file else { return Vec::new(); };
    let Ok(content) = std::fs::read_to_string(opam_entry.path()) else { return Vec::new(); };
    let declared = parse_opam_depends(&content);
    if declared.is_empty() { return Vec::new(); }

    let lib_dirs = ocaml_lib_dirs(project_root);
    let mut roots = Vec::new();
    for dep in &declared {
        for lib in &lib_dirs {
            let pkg_dir = lib.join(dep);
            if pkg_dir.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version: String::new(),
                    root: pkg_dir,
                    ecosystem: "ocaml",
                    package_id: None,
                });
                break;
            }
        }
    }
    debug!("OCaml: discovered {} external package roots", roots.len());
    roots
}

fn ocaml_lib_dirs(project_root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let local_opam = project_root.join("_opam").join("lib");
    if local_opam.is_dir() { dirs.push(local_opam); }
    if let Ok(switch) = std::env::var("OPAM_SWITCH_PREFIX") {
        let lib = PathBuf::from(switch).join("lib");
        if lib.is_dir() { dirs.push(lib); }
    }
    if let Some(home) = dirs::home_dir() {
        let opam = home.join(".opam");
        if opam.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&opam) {
                for e in entries.flatten() {
                    let lib = e.path().join("lib");
                    if lib.is_dir() { dirs.push(lib); }
                }
            }
        }
    }
    dirs
}

pub fn walk_ocaml_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_ocaml_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_ocaml_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_ocaml_dir_bounded(dir, root, dep, out, 0);
}

fn walk_ocaml_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue; };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "bench") || name.starts_with('.') { continue; }
            }
            walk_ocaml_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !(name.ends_with(".ml") || name.ends_with(".mli")) { continue; }
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
