// =============================================================================
// ecosystem/matlab_runtime.rs — MATLAB toolbox sources (stdlib-style ecosystem)
//
// MATLAB ships ~90 toolboxes (Statistics, Deep Learning, App Designer,
// Image Processing, Signal Processing, Optimization, Symbolic Math, …) as
// readable .m source under `$MATLABROOT/toolbox/`. User projects routinely
// use the toolbox APIs unqualified at runtime — `pdist2(...)`, `dlarray(...)`,
// `uibutton(parent, ...)`, `imshow(I)`, `fft(x)` — without imports.
//
// It walks the actual installed toolbox sources so the resolver can match
// toolbox functions / classes / methods to real symbols on disk. When no
// MATLAB is installed the probe finds nothing and the ecosystem stays inert.
//
// Probe order (each candidate is tried in turn; the first existing install
// root wins — none of these is required, discovery falls through to the
// next when a candidate is absent):
//   1. $MATLAB_ROOT — the standard env var MATLAB itself exports; some
//      users also set it in shell profiles.
//   2. `matlab -batch "disp(matlabroot)"` — query an installed binary.
//      Slow (multi-second startup) so guarded behind probe failure of
//      everything else.
//   3. Standard install paths on each OS:
//        Windows: `C:\Program Files\MATLAB\R20XXa|b\`
//        macOS:   `/Applications/MATLAB_R20XXa|b.app/`
//        Linux:   `/usr/local/MATLAB/R20XXa|b/`
//      Picks the lexicographically newest year/release.
//
// Walk: every `.m` file under `<root>/toolbox/`. Skips `tests/`, `private/`
// utility dirs, and language packs that don't contribute to user-callable
// API. Toolbox roots typically have ~30–50 sub-toolbox dirs; the full
// walk is bounded but non-trivial — a future demand-driven variant would
// load only the toolboxes the user's manifest references.
//
// Activation: any `.m` file in the project (`LanguagePresent("matlab")`).
// When MATLAB isn't installed, the probe returns empty and the ecosystem
// silently degrades.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rayon::prelude::*;
use tracing::debug;
use tree_sitter::{Node, Parser};

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("matlab-runtime");
const LEGACY_ECOSYSTEM_TAG: &str = "matlab-runtime";
const LANGUAGES: &[&str] = &["matlab"];

pub struct MatlabRuntimeEcosystem;

impl Ecosystem for MatlabRuntimeEcosystem {
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
        EcosystemActivation::LanguagePresent("matlab")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_matlab_toolbox()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_toolbox_tree(dep)
    }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_matlab_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }
}

impl ExternalSourceLocator for MatlabRuntimeEcosystem {
    fn ecosystem(&self) -> &'static str {
        LEGACY_ECOSYSTEM_TAG
    }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_matlab_toolbox()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_toolbox_tree(dep)
    }
}

fn discover_matlab_toolbox() -> Vec<ExternalDepRoot> {
    let Some(matlab_root) = probe_matlab_root() else {
        debug!("matlab-runtime: no MATLAB install probed");
        return Vec::new();
    };
    let toolbox = matlab_root.join("toolbox");
    if !toolbox.is_dir() {
        debug!(
            "matlab-runtime: install at {} has no toolbox/ subdir",
            matlab_root.display()
        );
        return Vec::new();
    }
    debug!("matlab-runtime: using {}", toolbox.display());
    vec![ExternalDepRoot {
        module_path: "matlab-runtime".to_string(),
        version: String::new(),
        root: toolbox,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn probe_matlab_root() -> Option<PathBuf> {
    if let Some(env_root) = std::env::var_os("MATLAB_ROOT") {
        let p = PathBuf::from(env_root);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Some(p) = probe_via_matlab_command() {
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Some(p) = probe_standard_install_paths() {
        return Some(p);
    }
    None
}

fn probe_via_matlab_command() -> Option<PathBuf> {
    // `matlab -batch "disp(matlabroot)"` runs a one-shot command and
    // exits. Non-trivial latency (~3–5s cold start) so this is the
    // last-resort probe.
    let output = Command::new("matlab")
        .args(["-batch", "disp(matlabroot)"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    for line in stdout.lines().rev() {
        let candidate = PathBuf::from(line.trim());
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

fn probe_standard_install_paths() -> Option<PathBuf> {
    // Each OS keeps releases in a parent dir; pick the lexicographically
    // newest entry (release names like `R2024a`, `R2024b` sort
    // chronologically — alphabetic-numeric works as a proxy).
    let parents = [
        "C:/Program Files/MATLAB",
        "C:/Program Files (x86)/MATLAB",
        "/Applications", // macOS — release dirs are `MATLAB_R20XXa.app`
        "/usr/local/MATLAB",
        "/opt/MATLAB",
    ];
    for parent_str in parents {
        let parent = Path::new(parent_str);
        let Ok(entries) = std::fs::read_dir(parent) else {
            continue;
        };
        let mut releases: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && looks_like_matlab_release_dir(p))
            .collect();
        releases.sort();
        if let Some(latest) = releases.into_iter().next_back() {
            // macOS: `MATLAB_R20XXa.app` — the install root is the .app
            // directly; toolbox lives inside it.
            return Some(latest);
        }
    }
    None
}

fn looks_like_matlab_release_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    // Matches `R2024a`, `R2024b`, `MATLAB_R2024a.app`, etc.
    name.starts_with('R') && name.contains("20")
        || name.starts_with("MATLAB_R") && name.contains("20")
}

fn walk_toolbox_tree(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_m_dir(&dep.root, &mut out, 0);
    out
}

fn walk_m_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    // Toolbox trees are deep — `toolbox/matlab/general/private/some.m` is
    // 4 levels in. Cap at 10 to keep walk cost predictable.
    if depth >= 10 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Skip test fixtures, language packs, examples, and private
                // helper dirs that don't form the user-callable surface.
                if matches!(
                    name,
                    "tests"
                        | "test"
                        | "fixtures"
                        | "examples"
                        | "demo"
                        | "demos"
                        | "private"
                        | "+private"
                        | "ja"
                        | "ja_JP"
                        | "ko"
                        | "ko_KR"
                        | "zh_CN"
                        | "zh_TW"
                        | "+internal"
                        | "internal"
                ) {
                    continue;
                }
                if name.starts_with('.') {
                    continue;
                }
            }
            walk_m_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".m") {
                continue;
            }
            // Skip Contents.m and other noise files.
            if name == "Contents.m" {
                continue;
            }
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:matlab:{display}"),
                absolute_path: path,
                language: "matlab",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------

fn build_matlab_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, crate::walker::WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_toolbox_tree(dep) {
            work.push((dep.module_path.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }
    let per_file: Vec<Vec<(String, String, std::path::PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_matlab_header(&src)
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

/// Header-only tree-sitter scan of a MATLAB source file. Each `.m` file
/// holds exactly one top-level `function_definition` or `class_definition`.
/// Methods inside a `methods` block are also indexed so chain walkers can
/// locate them by bare name.
pub(super) fn scan_matlab_header(source: &str) -> Vec<String> {
    let language = tree_sitter_matlab::LANGUAGE.into();
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
        collect_matlab_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_matlab_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        "class_definition" => {
            let class_name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(bytes).ok().map(String::from));
            if let Some(name) = class_name.as_ref() {
                out.push(name.clone());
            }
            // Index methods inside `methods` blocks so chain walkers can
            // resolve `obj.method_name` by bare name lookup.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "methods" {
                    let mut mc = child.walk();
                    for inner in child.children(&mut mc) {
                        if inner.kind() == "function_definition" {
                            if let Some(name_node) = inner.child_by_field_name("name") {
                                if let Ok(method) = name_node.utf8_text(bytes) {
                                    out.push(method.to_string());
                                    if let Some(cls) = class_name.as_ref() {
                                        out.push(format!("{cls}.{method}"));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<MatlabRuntimeEcosystem>> = OnceLock::new();
    LOCATOR
        .get_or_init(|| Arc::new(MatlabRuntimeEcosystem))
        .clone()
}

#[cfg(test)]
#[path = "matlab_runtime_tests.rs"]
mod tests;
