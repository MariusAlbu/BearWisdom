// =============================================================================
// ecosystem/nim_stdlib.rs — Nim standard library (stdlib ecosystem)
//
// The Nim stdlib ships as plain-text `.nim` source under the compiler's `lib/`
// directory (`system.nim`, `pure/strutils.nim`, `pure/os.nim`, …). The Nim
// extractor parses these directly, so this is a full source walk — no metadata
// synthesis needed.
//
// Probe order (degrades cleanly to empty when no toolchain is found):
//   1. $BEARWISDOM_NIM_SRC — explicit override pointing at the `lib/` dir.
//   2. `nim dump` — the compiler prints its library search paths on stderr;
//      walking up from one of them to the dir holding `system.nim` yields the
//      lib root.
//   3. The `nim` binary's sibling — `<dir-of-nim>/../lib`.
//
// Activation: `LanguagePresent("nim")` — every Nim module implicitly imports
// `system` and uses the stdlib (`strutils`, `sequtils`, `os`, …) as substrate.
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

pub const ID: EcosystemId = EcosystemId::new("nim-stdlib");
const TAG: &str = "nim-stdlib";
const LANGUAGES: &[&str] = &["nim"];

pub struct NimStdlibEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait
// ---------------------------------------------------------------------------

impl Ecosystem for NimStdlibEcosystem {
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
        EcosystemActivation::LanguagePresent("nim")
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["deprecated", "genode_cpp", "wrappers"]
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
        build_nim_symbol_index(dep_roots)
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for NimStdlibEcosystem {
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
    static LOCATOR: OnceLock<Arc<NimStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(NimStdlibEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover() -> Vec<ExternalDepRoot> {
    let Some(lib_dir) = probe_lib_dir() else {
        warn!("nim-stdlib: no Nim stdlib found (set BEARWISDOM_NIM_SRC or install Nim)");
        return Vec::new();
    };
    debug!("nim-stdlib: using {}", lib_dir.display());
    vec![ExternalDepRoot {
        module_path: "stdlib".to_string(),
        version: String::new(),
        root: lib_dir,
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

/// Probe the Nim `lib/` directory. A directory qualifies only when it contains
/// `system.nim` — the always-imported module present at the lib root.
fn probe_lib_dir() -> Option<PathBuf> {
    // 1. Explicit override.
    if let Some(val) = std::env::var_os("BEARWISDOM_NIM_SRC") {
        let p = PathBuf::from(val);
        if is_nim_lib(&p) {
            return Some(p);
        }
    }

    // 2. `nim dump` search paths → walk up to the lib root.
    if let Some(p) = probe_nim_dump() {
        return Some(p);
    }

    // 3. `nim` binary sibling — `<dir-of-nim>/../lib`.
    probe_nim_binary_sibling()
}

/// The Nim lib root is the directory that holds `system.nim`.
fn is_nim_lib(dir: &Path) -> bool {
    dir.join("system.nim").is_file()
}

/// Run `nim dump` and parse its stderr library search paths. Each path is an
/// absolute directory under the lib tree (`<lib>/pure`, `<lib>/core`); walking
/// up from any of them to the ancestor holding `system.nim` recovers the lib
/// root regardless of which subdir was listed first.
fn probe_nim_dump() -> Option<PathBuf> {
    for program in ["nim", "nim.exe"] {
        let Ok(out) = Command::new(program).arg("dump").output() else {
            continue;
        };
        if !out.status.success() {
            continue;
        }
        // Search paths are emitted on stderr.
        let stderr = String::from_utf8_lossy(&out.stderr);
        for line in stderr.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let p = PathBuf::from(trimmed);
            if !p.is_dir() {
                continue;
            }
            if let Some(root) = ascend_to_lib_root(&p) {
                return Some(root);
            }
        }
    }
    None
}

/// Walk up from `start` (a Nim lib subdir) to the nearest ancestor holding
/// `system.nim`, bounded to a few levels so an unrelated path can't escalate.
fn ascend_to_lib_root(start: &Path) -> Option<PathBuf> {
    let mut current = start;
    for _ in 0..4 {
        if is_nim_lib(current) {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
    None
}

/// Resolve the `nim` binary on PATH and check `<dir>/../lib`.
fn probe_nim_binary_sibling() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        for exe in ["nim", "nim.exe"] {
            if dir.join(exe).is_file() {
                let lib = dir.join("..").join("lib");
                if is_nim_lib(&lib) {
                    return Some(lib);
                }
            }
        }
    }
    None
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
            // Skip superseded modules and platform/FFI glue not used by typical
            // application code; hidden dirs are noise.
            if matches!(name, "deprecated" | "genode_cpp" | "wrappers") || name.starts_with('.') {
                continue;
            }
            walk_dir(&path, root, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".nim") && !name.ends_with(".nims") {
                continue;
            }
            let rel = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:nim:{rel}"),
                absolute_path: path,
                language: "nim",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol index — module-name → file map for demand-driven resolution
// ---------------------------------------------------------------------------

/// Build a `(stdlib, module) → file` index. A Nim import target is the bare
/// filename stem (`import strutils` → `strutils.nim`), so the filename is
/// authoritative — no parse needed.
pub(crate) fn build_nim_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut index = SymbolLocationIndex::new();
    for dep in dep_roots {
        for wf in walk(dep) {
            let Some(module) = wf
                .absolute_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            if module.is_empty() {
                continue;
            }
            index.insert(&dep.module_path, module.clone(), wf.absolute_path.clone());
            index.insert(module.clone(), module.clone(), wf.absolute_path.clone());
        }
    }
    index
}

#[cfg(test)]
#[path = "nim_stdlib_tests.rs"]
mod tests;
