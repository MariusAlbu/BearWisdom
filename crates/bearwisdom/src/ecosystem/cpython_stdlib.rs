// =============================================================================
// ecosystem/cpython_stdlib.rs — CPython stdlib (stdlib ecosystem)
//
// Probes the system Python install for its stdlib source tree. Strategy:
//   1. $BEARWISDOM_CPYTHON_STDLIB  → explicit dir
//   2. `python -c 'import sys; print(sys.prefix)'` + lib/pythonX.Y/
//   3. Fallback candidates: /usr/lib/pythonX.Y/, C:/PythonXY/Lib/
// Walks top-level .py files + subpackage dirs. Skips `test/`, `Lib/test/`,
// `turtledemo/`, and similar noise trees.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("cpython-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "cpython-stdlib";
const LANGUAGES: &[&str] = &["python"];

pub struct CpythonStdlibEcosystem;

impl Ecosystem for CpythonStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("python")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_cpython_stdlib()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_python_tree(dep)
    }
}

impl ExternalSourceLocator for CpythonStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_cpython_stdlib()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_python_tree(dep)
    }
}

fn discover_cpython_stdlib() -> Vec<ExternalDepRoot> {
    let Some(stdlib_dir) = probe_stdlib_dir() else {
        debug!("cpython-stdlib: no stdlib source found");
        return Vec::new();
    };
    debug!("cpython-stdlib: using {}", stdlib_dir.display());
    vec![ExternalDepRoot {
        module_path: "cpython-stdlib".to_string(),
        version: String::new(),
        root: stdlib_dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn probe_stdlib_dir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_CPYTHON_STDLIB") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p); }
    }
    if let Some(sys_prefix) = python_sys_prefix() {
        if let Some(stdlib) = stdlib_under_prefix(&sys_prefix) {
            return Some(stdlib);
        }
    }
    // Well-known fallbacks on common OSes.
    for candidate in [
        "/usr/lib/python3.12",
        "/usr/lib/python3.11",
        "/usr/lib/python3.10",
        "/usr/lib/python3.9",
        "/usr/local/lib/python3.12",
        "/usr/local/lib/python3.11",
        "C:/Python312/Lib",
        "C:/Python311/Lib",
        "C:/Python310/Lib",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_dir() { return Some(p); }
    }
    None
}

fn python_sys_prefix() -> Option<PathBuf> {
    for bin in ["python3", "python"] {
        let Ok(output) = Command::new(bin)
            .args(["-c", "import sys; print(sys.prefix)"])
            .output()
        else {
            continue;
        };
        if !output.status.success() { continue }
        let s = String::from_utf8(output.stdout).ok()?;
        let trimmed = s.trim();
        if trimmed.is_empty() { continue }
        let p = PathBuf::from(trimmed);
        if p.is_dir() { return Some(p); }
    }
    None
}

fn stdlib_under_prefix(prefix: &Path) -> Option<PathBuf> {
    // Unix: {prefix}/lib/python3.X
    if let Ok(lib_entries) = std::fs::read_dir(prefix.join("lib")) {
        for entry in lib_entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if name.starts_with("python3") && path.is_dir() {
                return Some(path);
            }
        }
    }
    // Windows: {prefix}/Lib
    let win = prefix.join("Lib");
    if win.is_dir() { return Some(win); }
    None
}

fn walk_python_tree(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= 14 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "test" | "tests" | "__pycache__" | "site-packages"
                        | "ensurepip" | "turtledemo" | "idlelib" | "tkinter"
                        | "dist-packages" | "unittest"
                ) {
                    continue;
                }
                if name.starts_with('.') { continue }
                // Skip `Lib/test/` on Windows which is huge and CI-style.
                if name == "Lib" && depth == 0 {
                    // fine — keep going
                }
            }
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".py") { continue }
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:python:{}", display),
                absolute_path: path,
                language: "python",
            });
        }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<CpythonStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(CpythonStdlibEcosystem)).clone()
}
