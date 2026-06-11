// =============================================================================
// ecosystem/ocaml_stdlib.rs — OCaml standard library (stdlib ecosystem)
//
// The OCaml stdlib ships as plain-text `.ml`/`.mli` source under the
// toolchain's stdlib directory (`<switch>/lib/ocaml/`): list.mli, string.mli,
// array.ml, … . The OCaml extractor parses these directly, so this is a full
// source walk — no metadata synthesis needed.
//
// Probe order (degrades cleanly to empty when no toolchain is found):
//   1. $BEARWISDOM_OCAML_SRC — explicit override pointing at the stdlib dir.
//   2. `ocamlc -where` — the authoritative stdlib path, when ocamlc is on PATH
//      (opam env active, or a system OCaml install).
//   3. `opam var lib` — `<lib>/ocaml` under the active opam switch.
//   4. opam root layout — `$OPAMROOT` / `~/.opam` / `%LOCALAPPDATA%/opam`,
//      enumerating switch directories for `<switch>/lib/ocaml`.
//
// Activation: `LanguagePresent("ocaml")` — every OCaml module uses the prelude
// (`List`, `String`, `Printf`, …) as language substrate.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tracing::{debug, warn};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("ocaml-stdlib");
const TAG: &str = "ocaml-stdlib";
const LANGUAGES: &[&str] = &["ocaml"];

pub struct OcamlStdlibEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait
// ---------------------------------------------------------------------------

impl Ecosystem for OcamlStdlibEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Stdlib
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("ocaml")
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["caml", "threads"]
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk(dep)
    }

    fn supports_reachability(&self) -> bool {
        true
    }
    fn uses_demand_driven_parse(&self) -> bool {
        true
    }
    fn is_workspace_global(&self) -> bool {
        true
    }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_ocaml_symbol_index(dep_roots)
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for OcamlStdlibEcosystem {
    fn ecosystem(&self) -> &'static str {
        TAG
    }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<OcamlStdlibEcosystem>> = OnceLock::new();
    LOCATOR
        .get_or_init(|| Arc::new(OcamlStdlibEcosystem))
        .clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover() -> Vec<ExternalDepRoot> {
    let Some(stdlib_dir) = probe_stdlib_dir() else {
        warn!(
            "ocaml-stdlib: no OCaml stdlib found \
             (set BEARWISDOM_OCAML_SRC, activate an opam switch, or install OCaml)"
        );
        return Vec::new();
    };
    debug!("ocaml-stdlib: using {}", stdlib_dir.display());
    vec![ExternalDepRoot {
        module_path: "stdlib".to_string(),
        version: String::new(),
        root: stdlib_dir,
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

/// Probe the OCaml stdlib directory via the documented fallback chain. A
/// directory qualifies only when it contains `list.mli` — the marker that this
/// is the OCaml stdlib rather than an unrelated lib dir.
fn probe_stdlib_dir() -> Option<PathBuf> {
    // 1. Explicit override.
    if let Some(val) = std::env::var_os("BEARWISDOM_OCAML_SRC") {
        let p = PathBuf::from(val);
        if is_ocaml_stdlib(&p) {
            return Some(p);
        }
    }

    // 2. `ocamlc -where` — authoritative when ocamlc is on PATH.
    if let Some(p) = probe_ocamlc_where() {
        if is_ocaml_stdlib(&p) {
            return Some(p);
        }
    }

    // 3. `opam var lib` → `<lib>/ocaml`.
    if let Some(p) = probe_opam_var_lib() {
        let ocaml = p.join("ocaml");
        if is_ocaml_stdlib(&ocaml) {
            return Some(ocaml);
        }
    }

    // 4. opam root layout enumeration.
    probe_opam_root_layout()
}

/// A directory is the OCaml stdlib when it holds the interface for the `List`
/// module — present in every OCaml install.
fn is_ocaml_stdlib(dir: &Path) -> bool {
    dir.join("list.mli").is_file() || dir.join("list.ml").is_file()
}

fn probe_ocamlc_where() -> Option<PathBuf> {
    for program in ["ocamlc", "ocamlc.opt", "ocamlc.exe"] {
        let Ok(out) = Command::new(program).arg("-where").output() else {
            continue;
        };
        if !out.status.success() {
            continue;
        }
        let raw = String::from_utf8_lossy(&out.stdout);
        let s = raw.trim();
        if s.is_empty() {
            continue;
        }
        let p = PathBuf::from(s);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

fn probe_opam_var_lib() -> Option<PathBuf> {
    for program in ["opam", "opam.exe"] {
        let Ok(out) = Command::new(program).args(["var", "lib"]).output() else {
            continue;
        };
        if !out.status.success() {
            continue;
        }
        let raw = String::from_utf8_lossy(&out.stdout);
        let s = raw.trim();
        if s.is_empty() {
            continue;
        }
        let p = PathBuf::from(s);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

/// Enumerate switch directories under the opam root and return the first
/// `<switch>/lib/ocaml` that looks like the stdlib. The opam root is taken from
/// `$OPAMROOT`, then the platform defaults (`~/.opam`, `%LOCALAPPDATA%/opam`).
fn probe_opam_root_layout() -> Option<PathBuf> {
    for root in opam_roots() {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        // Collect candidate switch dirs (each has lib/ocaml); newest name last.
        let mut switches: Vec<PathBuf> = entries
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.is_dir() && p.join("lib").join("ocaml").is_dir() {
                    Some(p)
                } else {
                    None
                }
            })
            .collect();
        switches.sort();
        for sw in switches.into_iter().rev() {
            let ocaml = sw.join("lib").join("ocaml");
            if is_ocaml_stdlib(&ocaml) {
                return Some(ocaml);
            }
        }
    }
    None
}

fn opam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(val) = std::env::var_os("OPAMROOT") {
        roots.push(PathBuf::from(val));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if let Some(ref h) = home {
        roots.push(h.join(".opam"));
    }
    // Windows opam installs default to %LOCALAPPDATA%/opam.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("opam"));
    }
    roots
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &dep.root, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, root: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // `caml/` holds C runtime headers; hidden dirs are noise.
            if name == "caml" || name.starts_with('.') {
                continue;
            }
            walk_dir(&path, root, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".ml") && !name.ends_with(".mli") {
                continue;
            }
            let rel = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:ocaml:{rel}"),
                absolute_path: path,
                language: "ocaml",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol index — module-name → file map for demand-driven resolution
// ---------------------------------------------------------------------------

/// Build a `(stdlib, ModuleName) → file` index. OCaml's compilation-unit module
/// name is the capitalized base filename (`list.mli` → `List`), so no parse is
/// needed — the filename is authoritative. The `.mli` interface is preferred
/// over the `.ml` implementation when both exist.
pub(crate) fn build_ocaml_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut index = SymbolLocationIndex::new();
    for dep in dep_roots {
        // First pass: register .mli interfaces. Second pass: register .ml only
        // when no .mli already covered that module — interface wins.
        let walked = walk(dep);
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for prefer_mli in [true, false] {
            for wf in &walked {
                let is_mli = wf.relative_path.ends_with(".mli");
                if is_mli != prefer_mli {
                    continue;
                }
                let Some(module) = module_name_from_path(&wf.absolute_path) else {
                    continue;
                };
                if !seen.insert(module.clone()) {
                    continue;
                }
                index.insert(&dep.module_path, module.clone(), wf.absolute_path.clone());
                index.insert(module.clone(), module.clone(), wf.absolute_path.clone());
            }
        }
    }
    index
}

/// OCaml module name = base filename with the first letter capitalized
/// (`stringLabels.mli` → `StringLabels`, `list.ml` → `List`).
fn module_name_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    if stem.is_empty() {
        return None;
    }
    let mut chars = stem.chars();
    let first = chars.next()?.to_uppercase().to_string();
    Some(format!("{first}{}", chars.as_str()))
}

#[cfg(test)]
#[path = "ocaml_stdlib_tests.rs"]
mod tests;
