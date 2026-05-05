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
        EcosystemActivation::All(&[
            EcosystemActivation::AlwaysOnPlatform(Platform::Unix),
            EcosystemActivation::Any(&[
                EcosystemActivation::LanguagePresent("c"),
                EcosystemActivation::LanguagePresent("cpp"),
            ]),
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
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }
}

fn discover_posix_include() -> Vec<ExternalDepRoot> {
    if cfg!(target_os = "windows") { return Vec::new() }
    let mut out = Vec::new();
    let candidates = [
        "/usr/include",
        "/usr/local/include",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_dir() {
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
        debug!("posix-headers: no /usr/include found");
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

/// Recursively enumerate `.h`/`.hpp`/`.hxx`/`.hh` files under `dir`,
/// computing each file's path relative to `root` — which is the user-
/// visible include path (e.g. `stdio.h`, `winrt/Windows.Foundation.h`).
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
        if !(name.ends_with(".h")
            || name.ends_with(".hpp")
            || name.ends_with(".hxx")
            || name.ends_with(".hh"))
        {
            continue;
        }
        let Ok(rel) = path.strip_prefix(root) else { continue };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        // Register each header at the path a compiler would name it through
        // THIS search root.  Don't also register the basename — that was
        // producing false matches where a user's `#include "async.h"`
        // (project-local, intended to resolve to sibling source) picked up
        // an unrelated `winrt/wrl/async.h` from the SDK. Multi-root
        // coverage (ucrt/, um/, winrt/, the version root, ...) gives
        // every `#include` form the right relative key without needing
        // a basename fallback.
        idx.insert(rel_str.clone(), rel_str.clone(), path.clone());
        // The C extractor's push_include emits Imports refs as
        //   target_name = basename, module = full path (e.g.
        //   `<openssl/bio.h>` → target=`bio.h`, module=`openssl/bio.h`).
        // The demand-loop's lookup is `locate(module, target_name)` which
        // looks up the key `(module, basename)`. Without a shadow registered
        // at that key the lookup misses and the file is never pulled.
        if let Some((dir, basename)) = rel_str.rsplit_once('/') {
            // Skip when basename == rel_str (no slash); we already have that.
            let _ = dir;
            idx.insert(rel_str.clone(), basename.to_string(), path.clone());
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
                idx.insert(lower.clone(), lower.clone(), path.clone());
                let base = lower.rsplit_once('/').map(|(_, b)| b.to_string());
                if let Some(b) = base {
                    idx.insert(lower, b, path.clone());
                }
            }
        }
    }
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
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn newest_sdk_version_picks_latest() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("10.0.22621.0/ucrt")).unwrap();
        fs::create_dir_all(tmp.path().join("10.0.26100.0/ucrt")).unwrap();
        fs::create_dir_all(tmp.path().join("wdf")).unwrap();
        let picked = newest_sdk_versions(tmp.path());
        assert_eq!(picked.len(), 1);
        assert!(picked[0].to_string_lossy().contains("10.0.26100.0"));
    }

    #[test]
    fn newest_sdk_version_ignores_unversioned_siblings() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("10.0.26100.0")).unwrap();
        fs::create_dir_all(tmp.path().join("shared")).unwrap();
        fs::create_dir_all(tmp.path().join("wdf")).unwrap();
        let picked = newest_sdk_versions(tmp.path());
        assert_eq!(picked.len(), 1);
        assert!(picked[0].to_string_lossy().contains("10.0.26100.0"));
    }

    #[test]
    fn header_index_registers_relative_path() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("stdio.h"), "int printf(const char*, ...);\n");
        write(&tmp.path().join("string.h"), "char* strcpy(char*, const char*);\n");
        let dep = make_root(tmp.path(), "test");
        let idx = build_c_header_index(&[dep]);
        assert!(!idx.is_empty());
        // `#include <stdio.h>` should locate stdio.h.
        assert!(idx.locate("stdio.h", "stdio.h").is_some());
        assert!(idx.locate("string.h", "string.h").is_some());
    }

    #[test]
    fn header_index_registers_only_relative_path_from_root() {
        // Two roots cover the same SDK layout from different angles: one
        // mounted at `winrt/` (so `Windows.Foundation.h` is at the root)
        // and one mounted at the version dir above it (so the file is at
        // `winrt/Windows.Foundation.h`).  This is the real discover_msvc
        // emission pattern.  Both `#include` spellings should resolve —
        // not via basename fallback, but via the matching root.
        let tmp = TempDir::new().unwrap();
        let version_root = tmp.path();
        write(&version_root.join("winrt/Windows.Foundation.h"), "/* header */\n");
        let winrt_root = make_root(&version_root.join("winrt"), "test");
        let version_dep = make_root(version_root, "test");
        let idx = build_c_header_index(&[winrt_root, version_dep]);
        // Reached via the winrt/ root (relative = `Windows.Foundation.h`).
        assert!(idx.locate("Windows.Foundation.h", "Windows.Foundation.h").is_some());
        // Reached via the version root (relative = `winrt/Windows.Foundation.h`).
        assert!(idx.locate("winrt/Windows.Foundation.h", "winrt/Windows.Foundation.h").is_some());
    }

    #[test]
    fn vcpkg_discovers_triplet_include_dirs() {
        // Lay out a fake vcpkg root with two triplets; only one with include/.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(&root.join("installed/x64-windows/include/openssl/bio.h"), "/* fake */\n");
        write(&root.join("installed/x64-linux/include/zlib.h"), "/* fake */\n");
        write(&root.join("installed/x86-windows/no-include-here.txt"), "ignored\n");

        std::env::set_var("VCPKG_ROOT", root);
        let dep_roots = discover_vcpkg_include();
        std::env::remove_var("VCPKG_ROOT");

        // Find the two triplet roots (x64-windows + x64-linux); x86-windows
        // is skipped because it has no include/.
        let triplet_dirs: Vec<String> = dep_roots
            .iter()
            .map(|r| r.root.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(
            triplet_dirs.iter().any(|p| p.ends_with("x64-windows/include")),
            "x64-windows/include not discovered; got {triplet_dirs:?}"
        );
        assert!(
            triplet_dirs.iter().any(|p| p.ends_with("x64-linux/include")),
            "x64-linux/include not discovered; got {triplet_dirs:?}"
        );
        assert!(
            !triplet_dirs.iter().any(|p| p.contains("x86-windows")),
            "x86-windows must be skipped (no include/); got {triplet_dirs:?}"
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_header_index_registers_lowercase_shadow_key() {
        // Windows SDK has mixed-case header names like `WinSock2.h` but
        // user code always writes `#include <winsock2.h>`. The HashMap
        // backing SymbolLocationIndex is case-sensitive, so we need a
        // lowercase shadow key. Without it, demand-driven walking misses
        // these headers entirely.
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("WinSock2.h"), "/* mixed-case sdk header */\n");
        let dep = make_root(tmp.path(), "test");
        let idx = build_c_header_index(&[dep]);
        // Original case still resolves.
        assert!(idx.locate("WinSock2.h", "WinSock2.h").is_some());
        // Lowercase form (what `#include <winsock2.h>` generates) also
        // resolves to the same file.
        assert!(
            idx.locate("winsock2.h", "winsock2.h").is_some(),
            "lowercase shadow key required for case-insensitive Windows lookup"
        );
    }

    #[test]
    fn header_index_does_not_basename_match_across_dirs() {
        // Regression: when only a deep-nested header exists, its basename
        // must NOT be registered — otherwise a user's project-local
        // `#include "async.h"` would wrongly pull a WinRT header.
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("wrl/async.h"), "/* winrt async */\n");
        let dep = make_root(tmp.path(), "test");
        let idx = build_c_header_index(&[dep]);
        assert!(idx.locate("wrl/async.h", "wrl/async.h").is_some());
        assert!(
            idx.locate("async.h", "async.h").is_none(),
            "basename-only lookup must not match a deeper-nested header",
        );
    }

    #[test]
    fn header_index_skips_non_header_files() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("foo.h"), "\n");
        write(&tmp.path().join("README.md"), "docs\n");
        write(&tmp.path().join("license.txt"), "text\n");
        let dep = make_root(tmp.path(), "test");
        let idx = build_c_header_index(&[dep]);
        assert!(idx.locate("foo.h", "foo.h").is_some());
        assert!(idx.locate("README.md", "README.md").is_none());
        assert!(idx.locate("license.txt", "license.txt").is_none());
    }

    #[test]
    fn resolve_header_finds_file_at_relative_path() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("stdio.h"), "\n");
        let dep = make_root(tmp.path(), "test");
        let found = resolve_header(&dep, "stdio.h").expect("should find stdio.h");
        assert!(found.absolute_path.ends_with("stdio.h"));
        assert_eq!(found.language, "c");
        assert!(found.relative_path.starts_with("ext:c:"));
    }

    #[test]
    fn resolve_header_falls_back_to_basename() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("winrt/Windows.Foundation.h"), "\n");
        let dep = make_root(tmp.path(), "test");
        // Ask for just the basename (no directory prefix). Scanner should
        // walk the tree and find it.
        let found = resolve_header(&dep, "Windows.Foundation.h").expect("basename fallback");
        assert!(found.absolute_path.ends_with("Windows.Foundation.h"));
    }

    #[test]
    fn resolve_header_returns_none_on_miss() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("foo.h"), "\n");
        let dep = make_root(tmp.path(), "test");
        assert!(resolve_header(&dep, "does-not-exist.h").is_none());
    }

    #[test]
    fn posix_ecosystem_declares_demand_driven() {
        assert!(PosixHeadersEcosystem.uses_demand_driven_parse());
        assert!(PosixHeadersEcosystem.supports_reachability());
    }

    #[test]
    fn posix_walk_root_is_empty_under_demand_driven() {
        let tmp = TempDir::new().unwrap();
        let dep = make_root(tmp.path(), POSIX_TAG);
        assert!(Ecosystem::walk_root(&PosixHeadersEcosystem, &dep).is_empty());
    }
}
