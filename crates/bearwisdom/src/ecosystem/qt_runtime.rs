// =============================================================================
// ecosystem/qt_runtime.rs — Qt SDK headers (cross-platform stdlib ecosystem)
//
// Probes a Qt5 or Qt6 install and surfaces its `include/` directory as an
// external dep root. Qt's layout puts every module in its own sub-directory
// (`QtCore/`, `QtGui/`, `QtWidgets/`, `QtNetwork/`, ...), each holding both
// camelcase class wrappers (`QObject`, `QString`) and real header files
// (`qobject.h`, `qstring.h`). Both forms are valid `#include` targets and we
// register both.
//
// Discovery probes (in order):
//   1. `BEARWISDOM_QT_DIR` env override
//   2. `QTDIR`, `Qt5_DIR`, `Qt6_DIR` env vars
//   3. Standard Qt online installer paths
//      (`C:/Qt/<ver>/<kit>/include`, `/Applications/Qt/<ver>/<kit>/include`)
//   4. aqtinstall paths (`~/Qt/<ver>/<kit>/include`)
//   5. System package manager paths (`/usr/include/qt5`,
//      `/usr/include/x86_64-linux-gnu/qt5`, MSYS2 `mingw64/include/qt5`,
//      Homebrew `/usr/local/opt/qt@5/include`)
//
// Demand-driven — `walk_root` returns `Vec::new()` and the symbol index
// maps every header's relative path to its absolute file. The Stage-2
// demand loop pulls only the headers a project actually `#include`s.
//
// Activation: any C/C++ project. Qt projects identify themselves via
// `find_package(Qt5/Qt6 ...)` in CMakeLists.txt or `QT +=` in .pro files,
// but for now we attach to all C/C++ projects and let the on-disk probe
// short-circuit when no Qt is installed.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("qt-runtime");
const TAG: &str = "qt-runtime";
const LANGUAGES: &[&str] = &["c", "cpp"];

pub struct QtRuntimeEcosystem;

impl Ecosystem for QtRuntimeEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("c"),
            EcosystemActivation::LanguagePresent("cpp"),
        ])
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_qt_include()
    }

    // Demand-driven — empty walk; the symbol index drives Stage-2 pulls.
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn supports_reachability(&self) -> bool { true }
    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_qt_header_index(dep_roots)
    }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        header: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        resolve_qt_header(dep, header).into_iter().collect()
    }

    fn resolve_symbol(&self, dep: &ExternalDepRoot, fqn: &str) -> Vec<WalkedFile> {
        resolve_qt_header(dep, fqn).into_iter().collect()
    }
}

impl ExternalSourceLocator for QtRuntimeEcosystem {
    fn ecosystem(&self) -> &'static str { TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_qt_include()
    }
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<QtRuntimeEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(QtRuntimeEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_qt_include() -> Vec<ExternalDepRoot> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut push_root = |dir: PathBuf, out: &mut Vec<ExternalDepRoot>| {
        if !dir.is_dir() { return; }
        let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        if seen.insert(canonical.clone()) {
            out.push(make_root(&dir));
        }
    };

    // Explicit override.
    if let Some(explicit) = std::env::var_os("BEARWISDOM_QT_DIR") {
        let p = PathBuf::from(explicit);
        // Either include/ or its parent.
        if p.join("include").is_dir() {
            push_root(p.join("include"), &mut out);
        } else if p.is_dir() {
            push_root(p, &mut out);
        }
    }

    // Standard Qt env vars.
    for var in ["QTDIR", "Qt6_DIR", "Qt5_DIR"] {
        if let Some(val) = std::env::var_os(var) {
            let p = PathBuf::from(val);
            if p.join("include").is_dir() {
                push_root(p.join("include"), &mut out);
            } else if p.ends_with("include") {
                push_root(p, &mut out);
            }
        }
    }

    // Per-host install layouts.
    for inc in autodetect_qt_include_dirs() {
        push_root(inc, &mut out);
    }

    if out.is_empty() {
        debug!("qt-runtime: no Qt install discovered");
    } else {
        debug!("qt-runtime: {} include root(s)", out.len());
    }
    out
}

/// Probe the conventional Qt install paths on each host platform.
/// Returns every `include/` directory found — caller dedupes by canonical path.
fn autodetect_qt_include_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();

    // ----- Windows -----
    if cfg!(target_os = "windows") {
        // Official online installer: C:/Qt/<version>/<kit>/include
        // <kit> is e.g. msvc2019_64, mingw_64, msvc2022_64, ...
        for qt_root in ["C:/Qt", "C:/Qt6"] {
            for ver in newest_subdirs(Path::new(qt_root)) {
                for kit in newest_subdirs(&ver) {
                    let include = kit.join("include");
                    if include.is_dir() { out.push(include); }
                }
            }
        }
        // MSYS2.
        for triple in ["mingw64", "mingw32", "ucrt64", "clang64"] {
            for qt_ver in ["qt5", "qt6"] {
                let p = PathBuf::from(format!("C:/msys64/{triple}/include/{qt_ver}"));
                if p.is_dir() { out.push(p); }
            }
        }
        // scoop install (less common but possible).
        if let Some(home) = std::env::var_os("USERPROFILE") {
            let scoop = PathBuf::from(home).join("scoop").join("apps");
            for app in ["qt", "qt5", "qt6"] {
                let p = scoop.join(app).join("current").join("include");
                if p.is_dir() { out.push(p); }
            }
        }
    }

    // ----- macOS -----
    if cfg!(target_os = "macos") {
        // Official online installer.
        for qt_root in ["/Applications/Qt"] {
            for ver in newest_subdirs(Path::new(qt_root)) {
                for kit in newest_subdirs(&ver) {
                    let include = kit.join("include");
                    if include.is_dir() { out.push(include); }
                }
            }
        }
        // Homebrew.
        for prefix in ["/opt/homebrew/opt/qt", "/usr/local/opt/qt",
                       "/opt/homebrew/opt/qt@5", "/usr/local/opt/qt@5",
                       "/opt/homebrew/opt/qt@6", "/usr/local/opt/qt@6"] {
            let p = PathBuf::from(prefix).join("include");
            if p.is_dir() { out.push(p); }
        }
    }

    // ----- Linux / unix -----
    if cfg!(target_os = "linux") || cfg!(target_os = "freebsd") {
        // Distro packaging.
        for prefix in [
            "/usr/include/qt5",
            "/usr/include/qt6",
            "/usr/include/x86_64-linux-gnu/qt5",
            "/usr/include/x86_64-linux-gnu/qt6",
            "/usr/include/aarch64-linux-gnu/qt5",
            "/usr/include/aarch64-linux-gnu/qt6",
            "/usr/local/include/qt5",
            "/usr/local/include/qt6",
        ] {
            let p = PathBuf::from(prefix);
            if p.is_dir() { out.push(p); }
        }
    }

    // aqtinstall lays Qt under ~/Qt/<version>/<kit>/include on every host.
    if let Some(home) = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
    {
        let aqt_root = PathBuf::from(home).join("Qt");
        for ver in newest_subdirs(&aqt_root) {
            for kit in newest_subdirs(&ver) {
                let include = kit.join("include");
                if include.is_dir() { out.push(include); }
            }
        }
    }

    out
}

/// Return the dirs immediately below `parent`, sorted descending by name.
/// Used to walk version directories where the highest-numbered dir wins.
fn newest_subdirs(parent: &Path) -> Vec<PathBuf> {
    if !parent.is_dir() { return Vec::new(); }
    let Ok(entries) = std::fs::read_dir(parent) else { return Vec::new(); };
    let mut subs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    subs.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    subs.into_iter().rev().collect()
}

fn make_root(dir: &Path) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: dir.to_string_lossy().into_owned(),
        version: String::new(),
        root: dir.to_path_buf(),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Symbol index — register every Qt header at every form a project might
// `#include` it under.
// ---------------------------------------------------------------------------

fn build_qt_header_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut idx = SymbolLocationIndex::new();
    for dep in dep_roots {
        collect_qt_headers_rec(&dep.root, &dep.root, &mut idx, 0);
    }
    if !idx.is_empty() {
        debug!("qt-runtime: indexed {} header paths", idx.len());
    }
    idx
}

fn collect_qt_headers_rec(
    root: &Path,
    dir: &Path,
    idx: &mut SymbolLocationIndex,
    depth: u32,
) {
    if depth >= 8 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            // Qt's `private/` subdirs hold internal-use headers — we skip
            // them to keep the index smaller; user code shouldn't include
            // them.
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == "private" { continue }
            }
            collect_qt_headers_rec(root, &path, idx, depth + 1);
            continue;
        }
        if !ft.is_file() { continue }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        // Two header forms in Qt:
        //   * Real headers ending in .h / .hpp / .hh
        //   * Camelcase class wrappers with no extension (`QObject`,
        //     `QStringList`) that simply `#include "qstringlist.h"`.
        let is_real_header = name.ends_with(".h")
            || name.ends_with(".hpp")
            || name.ends_with(".hxx")
            || name.ends_with(".hh");
        let is_qt_class_wrapper = name.starts_with('Q')
            && name.chars().nth(1).map(|c| c.is_ascii_uppercase()).unwrap_or(false)
            && !name.contains('.');
        if !is_real_header && !is_qt_class_wrapper { continue }

        let Ok(rel) = path.strip_prefix(root) else { continue };
        let rel_str = rel.to_string_lossy().replace('\\', "/");

        // Register at the relative path (`QtCore/QObject`, `QtCore/qobject.h`).
        idx.insert(rel_str.clone(), rel_str.clone(), path.clone());

        // Bare basename so `#include <QObject>` and `#include <qobject.h>`
        // also resolve. Required because the C extractor's push_include emits
        //   target_name = basename, module = full path
        // and the demand loop's lookup is `(module, target_name)`.
        if let Some((_, basename)) = rel_str.rsplit_once('/') {
            idx.insert(rel_str.clone(), basename.to_string(), path.clone());
            // Also register the basename as a standalone key — Qt projects
            // commonly write `#include <QObject>` (no module prefix), which
            // the C extractor emits as `target_name=QObject`, `module=QObject`.
            idx.insert(basename.to_string(), basename.to_string(), path.clone());
        }
    }
}

/// On-demand resolver — used when the chain walker asks for a Qt symbol/header
/// by name without being able to look it up in the symbol index.
fn resolve_qt_header(dep: &ExternalDepRoot, header: &str) -> Option<WalkedFile> {
    let candidate = dep.root.join(header);
    if candidate.is_file() {
        return Some(WalkedFile {
            relative_path: format!("ext:cpp:{}", candidate.to_string_lossy().replace('\\', "/")),
            absolute_path: candidate,
            language: "cpp",
        });
    }
    // Basename-only fallback.
    let mut stack = vec![dep.root.clone()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if ft.is_dir() { stack.push(path); continue; }
            if ft.is_file() && path.file_name().and_then(|n| n.to_str()) == Some(header) {
                return Some(WalkedFile {
                    relative_path: format!("ext:cpp:{}", path.to_string_lossy().replace('\\', "/")),
                    absolute_path: path,
                    language: "cpp",
                });
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "qt_runtime_tests.rs"]
mod tests;
