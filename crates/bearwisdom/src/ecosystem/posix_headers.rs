// =============================================================================
// ecosystem/posix_headers.rs — POSIX + MSVC C/C++ headers (stdlib ecosystem)
//
// Two ecosystems covering platform C/C++ headers:
//   * PosixHeadersEcosystem — /usr/include on unix-like systems.
//   * MsvcHeadersEcosystem  — $VCINSTALLDIR/include on Windows.
//
// Both activate when the project has C/C++ source files on the matching
// host platform.  On the wrong platform activation returns false and
// nothing probes.
//
// Demand-driven parsing
// ---------------------
// Both ecosystems declare `uses_demand_driven_parse = true` and return
// an empty `walk_root`.  `build_symbol_index` enumerates the header files
// under each dep root (no content parse) and registers each header under
// its `#include`-visible path, e.g. `stdio.h`, `windows.h`,
// `winrt/Windows.Foundation.h`.  The indexer's Stage-2 demand loop then
// pulls exactly the headers a user `#include`s — and their transitive
// includes as the pulled files are themselves parsed and their own
// Imports refs are queued.
//
// Why this matters
// ----------------
// The Windows SDK Include/ dir has five top-level children with wildly
// different footprints (`ucrt`: 66 headers, `um`: 1.5k, `shared`: 280,
// `winrt`: 400, `cppwinrt`: 1.4k — ~3.7k total).  The previous eager walk
// parsed every one of them on any Windows host with C/C++ source, even
// for Linux-first codebases (redis, sqlite, nginx) that need only a
// dozen POSIX-compatible stdlib headers.  The symptom was 4.7k spurious
// external files, 1.6M spurious symbols, and minutes of wasted work.
// Demand-driven matching the include graph keeps the external slice
// scoped to whatever the project actually reaches.
//
// The POSIX side has the same shape — `/usr/include` holds a few hundred
// to a few thousand headers depending on the distro + packages — so the
// same treatment applies.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
    Platform, SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

// ---------------------------------------------------------------------------
// PosixHeadersEcosystem
// ---------------------------------------------------------------------------

pub const POSIX_ID: EcosystemId = EcosystemId::new("posix-headers");
const POSIX_TAG: &str = "posix-headers";

pub struct PosixHeadersEcosystem;

impl Ecosystem for PosixHeadersEcosystem {
    fn id(&self) -> EcosystemId { POSIX_ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { &["c", "cpp"] }

    fn activation(&self) -> EcosystemActivation {
        // POSIX-style headers are available wherever the toolchain is
        // installed. On Unix this is `/usr/include`; on Windows the same
        // surface is provided by MSYS2 / mingw-w64 / WSL — discovery
        // probes all of them and returns roots only when a concrete
        // install is found. Activation just gates on "this is a C/C++
        // project"; the platform check moved into discovery.
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("c"),
            EcosystemActivation::LanguagePresent("cpp"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        // Precedence: when the project has a compile_commands.json the
        // exact -I paths it lists are ground truth — suppress this
        // heuristic walker so the wider /usr/include surface doesn't add
        // noise (false matches against unrelated system headers).
        if super::compile_commands::project_has_compile_commands_json(ctx.project_root) {
            return Vec::new();
        }
        discover_posix_include()
    }

    // Demand-driven: no eager walk. `build_symbol_index` enumerates headers
    // and registers their include-paths so Stage-2 can pull the right file
    // when a user `#include`s it.
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn supports_reachability(&self) -> bool { true }
    fn uses_demand_driven_parse(&self) -> bool { true }

    // /usr/include is a workspace-level "the OS provides this" fact.
    fn is_workspace_global(&self) -> bool { true }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_c_header_index(dep_roots)
    }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        header: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        resolve_header(dep, header).into_iter().collect()
    }

    fn resolve_symbol(&self, dep: &ExternalDepRoot, fqn: &str) -> Vec<WalkedFile> {
        resolve_header(dep, fqn).into_iter().collect()
    }
}

impl ExternalSourceLocator for PosixHeadersEcosystem {
    fn ecosystem(&self) -> &'static str { POSIX_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_posix_include()
    }
    fn locate_roots_for_package(
        &self,
        _workspace_root: &Path,
        _package_abs_path: &Path,
        _package_id: i64,
    ) -> Vec<ExternalDepRoot> {
        // Workspace-global: /usr/include doesn't depend on the package
        // path. Same roots regardless of caller.
        discover_posix_include()
    }
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }
}

fn discover_posix_include() -> Vec<ExternalDepRoot> {
    let mut out = Vec::new();

    // Native Unix-like host.
    #[cfg(not(target_os = "windows"))]
    {
        for c in ["/usr/include", "/usr/local/include"] {
            let p = PathBuf::from(c);
            if p.is_dir() {
                out.push(make_root(&p, POSIX_TAG));
            }
        }
    }

    // Windows host with a POSIX-compatible toolchain. Three commonly
    // installed sources, in preference order:
    //   * MSYS2 (`MSYSTEM_PREFIX/include`, `<msys-root>/usr/include`)
    //   * standalone mingw-w64 (`<mingw-root>/include`)
    //   * WSL distros (`\\wsl$\<distro>\usr\include`)
    // Each is probed via env vars / common install paths so the discovery
    // works on any Windows machine regardless of how the toolchain was
    // installed (scoop / chocolatey / official MSYS2 installer / WSL).
    #[cfg(target_os = "windows")]
    {
        for p in discover_mingw_msys_includes() {
            out.push(make_root(&p, POSIX_TAG));
        }
        for p in discover_wsl_includes() {
            out.push(make_root(&p, POSIX_TAG));
        }
    }

    if let Some(explicit) = std::env::var_os("BEARWISDOM_POSIX_INCLUDE") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            out.push(make_root(&p, POSIX_TAG));
        }
    }
    if out.is_empty() {
        debug!("posix-headers: no POSIX include directory probed");
    }
    out
}

#[cfg(target_os = "windows")]
fn discover_mingw_msys_includes() -> Vec<PathBuf> {
    use std::collections::HashSet;
    let mut roots: Vec<PathBuf> = Vec::new();

    // Roots advertised by the active shell (preferred — knows exactly
    // which subsystem is in use).
    if let Some(prefix) = std::env::var_os("MSYSTEM_PREFIX") {
        roots.push(PathBuf::from(prefix));
    }
    for env in ["MSYS2_ROOT", "MINGW_PREFIX", "MINGW_ROOT", "MINGW_HOME"] {
        if let Some(p) = std::env::var_os(env) { roots.push(PathBuf::from(p)); }
    }

    // Conventional install bases. The exact subdir name varies by
    // installer — scoop uses `<scoop>/apps/mingw/current/`, MSYS2 uses
    // `C:/msys64/`, choco uses `C:/ProgramData/mingw64/`. We collect
    // bases here and let the recursive include scanner below find the
    // right `include/` directory under each.
    roots.extend([
        PathBuf::from("C:/msys64"),
        PathBuf::from("C:/msys32"),
        PathBuf::from("C:/tools/msys64"),
    ]);
    if let Some(programdata) = std::env::var_os("ProgramData") {
        roots.push(PathBuf::from(&programdata).join("mingw64"));
        roots.push(PathBuf::from(&programdata).join("mingw32"));
    }
    if let Some(scoop) = std::env::var_os("SCOOP") {
        let apps = PathBuf::from(&scoop).join("apps");
        if let Ok(entries) = std::fs::read_dir(&apps) {
            for entry in entries.flatten() {
                if entry.file_type().map(|f| f.is_dir()).unwrap_or(false) {
                    let current = entry.path().join("current");
                    if current.is_dir() { roots.push(current); }
                }
            }
        }
    }

    // Walk each root looking for any `include/` dir containing `stdio.h`
    // (the C-standard libc header — its presence identifies a POSIX-
    // compatible header set). This sidesteps having to hardcode triplet
    // subdirs (`x86_64-w64-mingw32/include`, `mingw64/include`,
    // `usr/include`, ...) — we just discover whatever the install
    // actually shipped.
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for root in roots {
        if !root.is_dir() { continue }
        find_includes_with_stdio_h(&root, 4, &mut out, &mut seen);
    }
    out
}

#[cfg(target_os = "windows")]
fn find_includes_with_stdio_h(
    dir: &Path,
    depth: usize,
    out: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<PathBuf>,
) {
    if depth == 0 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() { continue }
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // Skip noise — toolchain output dirs, package managers, source
        // trees that aren't header roots.
        if matches!(name, "bin" | "lib" | "libexec" | "share" | "doc" | "etc"
            | "var" | "tmp" | "src" | "manifest" | "licenses")
        {
            continue;
        }
        if name == "include" {
            // Confirm: must contain `stdio.h` to be a libc header root.
            if path.join("stdio.h").is_file() && seen.insert(path.clone()) {
                out.push(path);
                continue;
            }
        }
        find_includes_with_stdio_h(&path, depth - 1, out, seen);
    }
}

#[cfg(target_os = "windows")]
fn discover_wsl_includes() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    // `\\wsl$\<distro>` and `\\wsl.localhost\<distro>` are the two
    // network paths Windows mounts WSL distros under. Iterate every
    // distro that exposes `/usr/include`.
    for prefix in [r"\\wsl$", r"\\wsl.localhost"] {
        let base = PathBuf::from(prefix);
        let Ok(entries) = std::fs::read_dir(&base) else { continue };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_dir() { continue }
            let candidate = entry.path().join("usr").join("include");
            if candidate.is_dir() && seen.insert(candidate.clone()) {
                out.push(candidate);
            }
        }
    }
    out
}

pub fn posix_shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PosixHeadersEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(PosixHeadersEcosystem)).clone()
}

// MSVC / Windows SDK headers live in `ecosystem/msvc_sdk.rs`. The shared
// helpers below (`make_root`, `newest_sdk_versions`, `build_c_header_index`,
// `resolve_header`) are pub(super) so msvc_sdk can reuse them.

// ---------------------------------------------------------------------------
// VcpkgHeadersEcosystem
// ---------------------------------------------------------------------------
//
// vcpkg is Microsoft's C/C++ package manager. When installed, it lays out
// every package's public headers under
// `<vcpkg_root>/installed/<triplet>/include/`, with the same `#include`
// path conventions a project would use (`<openssl/bio.h>`, `<libssh2.h>`,
// etc.). Each triplet (`x64-windows`, `x64-linux`, `arm64-osx`, ...) gets
// its own installed/ subtree with full headers for every installed pkg.
//
// We mirror the MSVC pattern: discover the triplet `include/` dir, register
// each header at its `#include`-visible relative path, and let the demand-
// driven loop pull whatever the project's own source actually `#include`s.

pub const VCPKG_ID: EcosystemId = EcosystemId::new("vcpkg-headers");
const VCPKG_TAG: &str = "vcpkg-headers";

pub struct VcpkgHeadersEcosystem;

impl Ecosystem for VcpkgHeadersEcosystem {
    fn id(&self) -> EcosystemId { VCPKG_ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { &["c", "cpp"] }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("c"),
            EcosystemActivation::LanguagePresent("cpp"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        // Precedence: ground truth from compile_commands.json beats vcpkg
        // discovery — the build will already have the right -I paths.
        if super::compile_commands::project_has_compile_commands_json(ctx.project_root) {
            return Vec::new();
        }
        discover_vcpkg_include()
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn supports_reachability(&self) -> bool { true }
    fn uses_demand_driven_parse(&self) -> bool { true }

    // vcpkg's `installed/<triplet>/include` is workspace-level — one
    // installation supplies headers for every translation unit in the build.
    fn is_workspace_global(&self) -> bool { true }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_c_header_index(dep_roots)
    }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        header: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        resolve_header(dep, header).into_iter().collect()
    }

    fn resolve_symbol(&self, dep: &ExternalDepRoot, fqn: &str) -> Vec<WalkedFile> {
        resolve_header(dep, fqn).into_iter().collect()
    }
}

impl ExternalSourceLocator for VcpkgHeadersEcosystem {
    fn ecosystem(&self) -> &'static str { VCPKG_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_vcpkg_include()
    }
    fn locate_roots_for_package(
        &self,
        _workspace_root: &Path,
        _package_abs_path: &Path,
        _package_id: i64,
    ) -> Vec<ExternalDepRoot> {
        discover_vcpkg_include()
    }
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }
}

/// Locate vcpkg's per-triplet `include/` dirs. Discovery order:
///   1. `BEARWISDOM_VCPKG_INCLUDE` env override (single dir).
///   2. `VCPKG_ROOT` env var.
///   3. Common install paths: `F:/Work/Projects/vcpkg`, `C:/vcpkg`,
///      `~/vcpkg`, `/usr/local/share/vcpkg`.
///
/// vcpkg keeps a per-triplet directory under `installed/`. We emit one
/// `ExternalDepRoot` per discovered triplet's `include/` so the relative
/// path key in the symbol index matches a project's `#include` syntax.
fn discover_vcpkg_include() -> Vec<ExternalDepRoot> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_VCPKG_INCLUDE") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            return vec![make_root(&p, VCPKG_TAG)];
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(root) = std::env::var_os("VCPKG_ROOT") {
        candidates.push(PathBuf::from(root));
    }
    // Common install spots (cheap to stat; only existing ones turn into roots).
    let defaults: &[&str] = if cfg!(target_os = "windows") {
        &[
            "F:/Work/Projects/vcpkg",
            "C:/vcpkg",
            "C:/dev/vcpkg",
            "C:/tools/vcpkg",
        ]
    } else {
        &[
            "/usr/local/share/vcpkg",
            "/opt/vcpkg",
        ]
    };
    for d in defaults {
        candidates.push(PathBuf::from(d));
    }
    if let Some(home) = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
    {
        candidates.push(PathBuf::from(home).join("vcpkg"));
    }

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for cand in candidates {
        if !cand.is_dir() {
            continue;
        }
        let installed = cand.join("installed");
        let Ok(entries) = std::fs::read_dir(&installed) else { continue };
        for e in entries.flatten() {
            let triplet_dir = e.path();
            let include = triplet_dir.join("include");
            if !include.is_dir() {
                continue;
            }
            // Dedup across candidates that point at the same physical dir.
            let canonical = include.canonicalize().unwrap_or_else(|_| include.clone());
            if !seen.insert(canonical.clone()) {
                continue;
            }
            out.push(make_root(&include, VCPKG_TAG));
        }
    }
    if out.is_empty() {
        debug!("vcpkg-headers: no installed triplets discovered");
    }
    out
}

pub fn vcpkg_shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<VcpkgHeadersEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(VcpkgHeadersEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub(super) fn make_root(dir: &Path, tag: &'static str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: dir.to_string_lossy().into_owned(),
        version: String::new(),
        root: dir.to_path_buf(),
        ecosystem: tag,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

/// Pick the newest `10.0.*` version directory under a Windows SDK Include
/// root. The SDK nests versioned dirs (e.g. `Include/10.0.26100.0/`),
/// each with its own `ucrt/`, `um/`, etc. tree. Walking every installed
/// version would multiply the symbol-index size with no benefit — the
/// newest is the one the compiler picks unless the user asks otherwise.
pub(super) fn newest_sdk_versions(include: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(include) else { return Vec::new() };
    let mut versions: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|s| s.starts_with("10."))
        })
        .collect();
    versions.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    versions.into_iter().next_back().into_iter().collect()
}

/// Walk every dep root and register each header at its `#include`-visible
/// relative path. The symbol index uses `(module_path, symbol_name)` →
/// file; for headers we set both key slots to the header's relative path
/// so `#include <stdio.h>` (emitted as `target_name="stdio.h"`,
/// `module="stdio.h"` by the C extractor) resolves directly.
pub(super) fn build_c_header_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut idx = SymbolLocationIndex::new();
    for dep in dep_roots {
        collect_headers_rec(&dep.root, &dep.root, &mut idx, 0);
    }
    if !idx.is_empty() {
        debug!("c-headers: indexed {} header paths", idx.len());
    }
    idx
}

/// Recursively enumerate header files under `dir`, computing each file's
/// path relative to `root` — which is the user-visible include path
/// (e.g. `stdio.h`, `winrt/Windows.Foundation.h`).
///
/// Two file shapes count as headers:
///
///   * Real headers: `.h` / `.hpp` / `.hxx` / `.hh` extensions.
///   * Forwarding wrappers: extensionless files whose body is a single
///     `#include "x.h"` or `#include <x.h>` directive (Qt's `QObject`
///     pattern, plus a handful of older C++ libs that ship the same shape).
///     Detected by content, not by name — the file's first non-comment
///     non-whitespace tokens must be `#include` followed by a
///     `"..."` or `<...>` target. Anything else (LICENSE, README,
///     Makefile) fails the check and is skipped.
fn collect_headers_rec(
    root: &Path,
    dir: &Path,
    idx: &mut SymbolLocationIndex,
    depth: u32,
) {
    if depth >= 10 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Historical skip — vendor test fixtures sometimes drop noisy
                // sub-dirs inside /usr/include. Harmless on SDK trees.
                if matches!(name, "tests" | "test") { continue }
            }
            collect_headers_rec(root, &path, idx, depth + 1);
            continue;
        }
        if !ft.is_file() { continue }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let is_real_header = name.ends_with(".h")
            || name.ends_with(".hpp")
            || name.ends_with(".hxx")
            || name.ends_with(".hh");
        // Wrapper candidates: extensionless files with no dot at all.
        // The `dot` check filters out `LICENSE.txt`, `version.in`, etc. —
        // a true wrapper has no extension. Size cap avoids reading large
        // text files; real wrappers are <100 bytes.
        let is_wrapper_candidate = !is_real_header
            && !name.contains('.')
            && entry.metadata().map(|m| m.len() <= 1024).unwrap_or(false);
        // Detect AND extract the forwarding target in one read. When this
        // returns Some(target), the wrapper points all its index keys at
        // the resolved real header rather than at itself — see why below.
        let wrapper_target = if is_wrapper_candidate {
            wrapper_forward_target(&path)
        } else {
            None
        };
        let is_wrapper = wrapper_target.is_some();
        // Modern C++ stdlib headers (`<vector>`, `<memory>`, `<string>`,
        // `<iostream>`) are extensionless files containing the full
        // template definitions, not forwarding wrappers. They fail the
        // wrapper check (too large) and the extension check (none).
        // Recognize them by name shape: lowercase identifier, no dots
        // or dashes — matches C++14/17/20 stdlib conventions
        // (`unordered_map`, `string_view`, `condition_variable`) and
        // doesn't collide with files like `LICENSE`, `README`,
        // `Makefile`, or arbitrary capitalized identifiers.
        let is_cpp_stdlib_header = !is_real_header
            && !is_wrapper
            && !name.contains('.')
            && is_cpp_stdlib_header_name(name);
        if !is_real_header && !is_wrapper && !is_cpp_stdlib_header { continue }

        let Ok(rel) = path.strip_prefix(root) else { continue };
        let rel_str = rel.to_string_lossy().replace('\\', "/");

        // For wrappers we register the alias names pointing at the *real*
        // header, not at the wrapper file itself. Two reasons:
        //   1. The seed phase's `make_walked_file` does language detection
        //      by file extension only — extensionless wrapper files like
        //      Qt's `QObject` / `QTest` are silently dropped because no
        //      extension matches a registered language. Pointing keys at
        //      `qobject.h` / `qtest.h` instead bypasses the detection cliff
        //      entirely; the demand loop parses the `.h` directly.
        //   2. The real symbols (the class declarations, the macros) live
        //      in the `.h`. Pointing aliases there means the chain reaches
        //      them in one fewer hop, regardless of language detection.
        let target_path = match wrapper_target {
            Some(ref tgt) => {
                // Resolve target relative to the wrapper's directory.
                // Wrappers conventionally include a sibling header
                // (`#include "qobject.h"`); fall back to the wrapper itself
                // if the target file isn't on disk.
                let wrapper_dir = path.parent().unwrap_or(&path);
                let resolved = wrapper_dir.join(tgt);
                if resolved.is_file() { resolved } else { path.clone() }
            }
            None => path.clone(),
        };

        // Register each header at the path a compiler would name it through
        // THIS search root. Don't register the bare basename for real
        // headers — that was producing false matches where a user's
        // `#include "async.h"` (project-local, intended to resolve to a
        // sibling source) picked up an unrelated `winrt/wrl/async.h` from
        // the SDK. Multi-root coverage (ucrt/, um/, winrt/, the version
        // root, ...) gives every `#include` form the right relative key
        // without needing a basename fallback.
        idx.insert(rel_str.clone(), rel_str.clone(), target_path.clone());
        // The C extractor's push_include emits Imports refs as
        //   target_name = basename, module = full path (e.g.
        //   `<openssl/bio.h>` → target=`bio.h`, module=`openssl/bio.h`).
        // The demand-loop's lookup is `locate(module, target_name)` which
        // looks up the key `(module, basename)`. Without a shadow registered
        // at that key the lookup misses and the file is never pulled.
        if let Some((_dir, basename)) = rel_str.rsplit_once('/') {
            idx.insert(rel_str.clone(), basename.to_string(), target_path.clone());
        }
        // Wrapper files additionally register their basename as a
        // standalone key so `#include <QObject>` (which the C extractor
        // emits as `target_name="QObject", module="QObject"`) resolves.
        // Safe because a wrapper's basename is unique within an SDK include
        // tree — its content forwards to the real `.h`, where the actual
        // class lives.
        if is_wrapper {
            let basename = rel_str.rsplit_once('/').map(|(_, b)| b).unwrap_or(&rel_str);
            idx.insert(basename.to_string(), basename.to_string(), target_path.clone());
        }
        // Windows filesystem is case-insensitive but the SymbolLocationIndex
        // is a case-sensitive HashMap. On Windows, a project's
        // `#include <winsock2.h>` (lowercase) won't match the SDK's
        // on-disk `WinSock2.h` without a lowercase shadow key.
        // Insert it so demand-driven walks still pull the file.
        #[cfg(target_os = "windows")]
        {
            let lower = rel_str.to_ascii_lowercase();
            if lower != rel_str {
                idx.insert(lower.clone(), lower.clone(), target_path.clone());
                let base = lower.rsplit_once('/').map(|(_, b)| b.to_string());
                if let Some(b) = base {
                    idx.insert(lower, b, target_path.clone());
                }
            }
        }
    }
}

/// True for filenames matching the C++ stdlib extensionless header
/// convention: starts with a lowercase letter, contains only lowercase
/// ASCII letters, digits, and underscores. Catches `vector`, `memory`,
/// `string`, `unordered_map`, `string_view`, `condition_variable`,
/// `cstdio`, `cmath`. Excludes `LICENSE`, `README`, `Makefile`, version
/// files, and capitalized identifiers (Win32 SDK headers like
/// `EnterCriticalSection` are .h files, never extensionless).
///
/// Length floor of 2 chars filters single-letter junk. No upper bound
/// — header names can run up to ~30 chars (`condition_variable`).
fn is_cpp_stdlib_header_name(name: &str) -> bool {
    if name.len() < 2 { return false }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() { return false }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Public face of `is_cpp_stdlib_header_name` so the indexer's
/// language-detection fallback (`make_walked_file`) can recognize
/// extensionless C++ stdlib headers and route them to the C++
/// extractor. Without this, files pulled by the demand loop
/// (`vector`, `memory`, `string`, ...) get silently dropped because
/// `language_by_extension` returns None for them.
pub fn is_extensionless_cpp_stdlib_header(name: &str) -> bool {
    !name.contains('.') && is_cpp_stdlib_header_name(name)
}

#[cfg(test)]
pub(super) fn _test_is_cpp_stdlib_header_name(name: &str) -> bool {
    is_cpp_stdlib_header_name(name)
}

/// Returns the include target if `path` is a forwarding header — its first
/// non-comment non-whitespace bytes form a single `#include "x"` or
/// `#include <x>` directive. Used to detect extensionless C++ class
/// wrappers like Qt's `QObject`/`QStringList`/`QList` without naming them.
///
/// Reads up to 1KB. Strips `// ...` line comments and `/* ... */` block
/// comments, then accepts the first `#include` token followed by a
/// quoted-or-bracketed target.
fn wrapper_forward_target(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 1024];
    let n = f.read(&mut buf).ok()?;
    let text = std::str::from_utf8(&buf[..n]).ok()?;
    parse_include_target(text)
}

#[cfg(test)]
pub(super) fn _test_parse_include_target(text: &str) -> Option<String> {
    parse_include_target(text)
}

/// Parse the first `#include` directive after skipping leading whitespace
/// and C/C++ comments. Returns the include target without quotes/brackets.
/// Returns None for files that have non-comment non-include content first
/// (regular headers, license files, makefiles, etc.).
fn parse_include_target(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b' ' || b == b'\t' || b == b'\r' || b == b'\n' {
            i += 1;
            continue;
        }
        // // line comment
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
            continue;
        }
        // /* block comment */
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < bytes.len() { i += 2; }
            continue;
        }
        // Must be a `#` next, then the literal token `include`.
        if b != b'#' { return None }
        i += 1;
        // Skip whitespace between # and `include` — `# include` is legal C.
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') { i += 1; }
        let kw = b"include";
        if i + kw.len() > bytes.len() || &bytes[i..i + kw.len()] != kw { return None }
        i += kw.len();
        // Mandatory whitespace before target.
        if i >= bytes.len() || !(bytes[i] == b' ' || bytes[i] == b'\t') { return None }
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') { i += 1; }
        if i >= bytes.len() { return None }
        let (open, close) = match bytes[i] {
            b'"' => (b'"', b'"'),
            b'<' => (b'<', b'>'),
            _ => return None,
        };
        let _ = open;
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != close && bytes[i] != b'\n' { i += 1; }
        if i >= bytes.len() || bytes[i] != close { return None }
        let target = &text[start..i];
        if target.is_empty() { return None }
        return Some(target.to_string());
    }
    None
}

/// On-demand header resolution for `resolve_import` / `resolve_symbol`.
/// Tries the full relative path first, then falls back to the basename so
/// both `<winrt/Foo.h>` and a basename-only demand (generated by the
/// chain walker) land on the same file.
pub(super) fn resolve_header(dep: &ExternalDepRoot, header: &str) -> Option<WalkedFile> {
    let candidate = dep.root.join(header);
    if candidate.is_file() {
        return Some(WalkedFile {
            relative_path: format!("ext:c:{}", candidate.to_string_lossy().replace('\\', "/")),
            absolute_path: candidate,
            language: "c",
        });
    }
    // Basename-only fallback: scan dep.root recursively for a matching file.
    // Rare path — only hit when the demand loop tries a bare symbol name.
    let mut stack = vec![dep.root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if ft.is_dir() {
                stack.push(path);
                continue;
            }
            if ft.is_file() && path.file_name().and_then(|n| n.to_str()) == Some(header) {
                return Some(WalkedFile {
                    relative_path: format!("ext:c:{}", path.to_string_lossy().replace('\\', "/")),
                    absolute_path: path,
                    language: "c",
                });
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "posix_headers_tests.rs"]
mod tests;
