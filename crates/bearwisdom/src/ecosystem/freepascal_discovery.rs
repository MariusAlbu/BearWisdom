// =============================================================================
// ecosystem/freepascal_discovery — locate the Lazarus/FPC install's dep roots
//
// Probes the Lazarus install root, then surfaces the LCL, bundled components,
// and the FPC RTL + stdlib package trees as ExternalDepRoots. The host
// target's RTL tree and its Makefile-declared shared families register first;
// every other platform tree follows, so the location index's first-writer-wins
// keeps host winners for unit names all platforms declare while platform-only
// units stay locatable for cross-platform project sources.
// =============================================================================

use std::path::{Path, PathBuf};

use tracing::debug;

use super::LEGACY_ECOSYSTEM_TAG;
use crate::ecosystem::externals::ExternalDepRoot;
pub(crate) fn discover_freepascal_roots() -> Vec<ExternalDepRoot> {
    let Some(lazarus_root) = lazarus_install_root() else {
        debug!("No Lazarus install discovered; skipping FreePascal runtime");
        return Vec::new();
    };
    debug!("FreePascal runtime: scanning {}", lazarus_root.display());

    let mut roots = Vec::new();

    // LCL — top-level Pascal source tree, no nested package layout.
    let lcl = lazarus_root.join("lcl");
    if lcl.is_dir() {
        roots.push(ExternalDepRoot {
            module_path: "lcl".to_string(),
            version: String::new(),
            root: lcl,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }

    // Lazarus-bundled components — each is its own package directory.
    // We push the parent so a single walk covers all of them; the walker
    // includes .pas/.pp from any depth.
    let components = lazarus_root.join("components");
    if components.is_dir() {
        roots.push(ExternalDepRoot {
            module_path: "lazarus-components".to_string(),
            version: String::new(),
            root: components,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }

    // FPC RTL + FCL packages — versioned subdirectory.
    let fpc_dir = lazarus_root.join("fpc");
    if let Some(ver_dir) = first_subdir(&fpc_dir) {
        let source = ver_dir.join("source");
        if source.is_dir() {
            // RTL host target — pick a single platform tree to avoid
            // indexing rtl/aix, rtl/amiga, etc. on a Windows machine.
            let rtl = source.join("rtl");
            for target in rtl_host_targets() {
                let candidate = rtl.join(target);
                if candidate.is_dir() {
                    let shared = shared_rtl_dirs(&candidate);
                    roots.push(ExternalDepRoot {
                        module_path: format!("fpc-rtl-{target}"),
                        version: String::new(),
                        root: candidate,
                        ecosystem: LEGACY_ECOSYSTEM_TAG,
                        package_id: None,
                        requested_imports: Vec::new(),
                    });
                    // FPC's own build system (this target's Makefile.fpc)
                    // additionally compiles some of its units out of a shared
                    // family directory it never copies locally — sysutils.pp
                    // for win64/win32 lives only under `win`, for
                    // linux/darwin/freebsd only under `unix`. Register those
                    // too so their units are reachable.
                    for name in shared {
                        let shared_root = rtl.join(&name);
                        if shared_root.is_dir() {
                            roots.push(ExternalDepRoot {
                                module_path: format!("fpc-rtl-{name}"),
                                version: String::new(),
                                root: shared_root,
                                ecosystem: LEGACY_ECOSYSTEM_TAG,
                                package_id: None,
                                requested_imports: Vec::new(),
                            });
                        }
                    }
                    break;
                }
            }
            // inc — platform-independent RTL declarations (heap.inc,
            // mathh.inc, systemh.inc, generic.inc, etc.). These are included
            // by the platform-specific system.pp via {$I} directives; the
            // walker indexes them directly so that compiler-intrinsic
            // declarations (GetMem, FreeMem, Abs, Sqr, Move, ...) are
            // present in the symbol index without requiring a preprocessor.
            let inc = rtl.join("inc");
            if inc.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: "fpc-rtl-inc".to_string(),
                    version: String::new(),
                    root: inc,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
            }
            // objpas — common units (Classes, SysUtils, Math, Variants,
            // ...). Loaded on every target.
            let objpas = rtl.join("objpas");
            if objpas.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: "fpc-rtl-objpas".to_string(),
                    version: String::new(),
                    root: objpas,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
            }
            // Non-host platform trees — registered AFTER the host target and
            // its shared families, so the location index's first-writer-wins
            // keeps host winners for unit names every platform declares while
            // platform-only units (BaseUnix, unixtype, cocoaall) stay
            // locatable for a project's cross-platform sources. Demand-gated:
            // a tree contributes files only when a ref reaches into it.
            {
                let mut covered: Vec<String> = vec!["inc".into(), "objpas".into()];
                if let Some(host) = rtl_host_targets().iter().find(|t| rtl.join(t).is_dir()) {
                    covered.push((*host).to_string());
                    covered.extend(shared_rtl_dirs(&rtl.join(host)));
                }
                if let Ok(entries) = std::fs::read_dir(&rtl) {
                    let mut names: Vec<String> = entries
                        .flatten()
                        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                        .filter_map(|e| e.file_name().to_str().map(str::to_string))
                        .filter(|n| !covered.iter().any(|c| c == n))
                        .collect();
                    names.sort_unstable();
                    for name in names {
                        roots.push(ExternalDepRoot {
                            module_path: format!("fpc-rtl-{name}"),
                            version: String::new(),
                            root: rtl.join(&name),
                            ecosystem: LEGACY_ECOSYSTEM_TAG,
                            package_id: None,
                            requested_imports: Vec::new(),
                        });
                    }
                }
            }
            // FPC stdlib packages (winunits-base, fcl-*, gtk2, cocoaint, x11, ...).
            // Each package under packages/<name>/src/ is emitted as a separate
            // ExternalDepRoot so the module_path namespaces package symbols
            // distinctly (fpc-pkg-winunits-base, fpc-pkg-fcl-base, ...).
            //
            // Platform-specific packages that are large and irrelevant on the
            // current host are skipped: cocoaint on non-macOS, x11/gtk2 on
            // Windows/macOS, winunits-* on Linux/macOS. Cross-platform packages
            // (fcl-*, rtl-*, paszlib, hash, ...) are always walked.
            let packages_dir = source.join("packages");
            if packages_dir.is_dir() {
                emit_package_roots(&packages_dir, &mut roots);
            }
        }
    }

    debug!("FreePascal runtime: {} roots", roots.len());
    roots
}

/// Return the host platform's expected FPC RTL subdirectory name(s).
/// Listed in fallback order — first match wins.
pub(crate) fn rtl_host_targets() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        if cfg!(target_pointer_width = "64") { &["win64", "win32", "win"] }
        else { &["win32", "win"] }
    } else if cfg!(target_os = "linux") {
        &["linux", "unix"]
    } else if cfg!(target_os = "macos") {
        &["darwin", "macos", "unix"]
    } else if cfg!(target_os = "freebsd") {
        &["freebsd", "bsd", "unix"]
    } else {
        &["unix"]
    }
}

/// Sibling RTL directories `target_dir`'s own unit sources depend on, as
/// declared by FPC's build system in `target_dir/Makefile.fpc`. FPC's
/// per-platform units share a family directory the target-specific tree
/// doesn't duplicate — win64/win32 compile `sysutils.pp` out of `win`
/// (`WINDIR=../win`), linux/darwin/freebsd compile theirs out of `unix`
/// (`UNIXINC=$(RTL)/unix`), and darwin/freebsd also pull `bsd`
/// (`BSDINC=$(RTL)/bsd`). Returned as directory names relative to the `rtl`
/// root — the caller resolves and existence-checks each one.
///
/// Parses `NAME=../dir` and `NAME=$(RTL)/dir` assignment lines, the two
/// forms FPC's Makefile.fpc generator uses for this. An assignment whose
/// value nests more than one path segment (`WININC=../win/wininc`, already
/// covered once `win` itself is walked) or has none (`RTL=..`, the
/// self-reference every Makefile.fpc carries) contributes nothing.
pub(crate) fn shared_rtl_dirs(target_dir: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(target_dir.join("Makefile.fpc")) else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for line in content.lines() {
        let Some((_key, value)) = line.trim().split_once('=') else { continue };
        let rel = value.strip_prefix("../").or_else(|| value.strip_prefix("$(RTL)/"));
        let Some(rel) = rel else { continue };
        if rel.is_empty() || rel.contains(['/', '\\']) {
            continue;
        }
        if !dirs.iter().any(|d: &String| d == rel) {
            dirs.push(rel.to_string());
        }
    }
    dirs
}

pub(crate) fn first_subdir(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() { return None }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect();
    entries.sort();
    entries.into_iter().next_back() // newest version wins
}

pub(crate) fn lazarus_install_root() -> Option<PathBuf> {
    // Explicit override.
    if let Ok(val) = std::env::var("BEARWISDOM_LAZARUS_DIR") {
        let p = PathBuf::from(val);
        if p.is_dir() { return Some(p) }
    }
    // Standard Lazarus env (set by the IDE installer).
    if let Ok(val) = std::env::var("LAZARUS_DIR") {
        let p = PathBuf::from(val);
        if p.is_dir() { return Some(p) }
    }

    // Scoop install on Windows (most common dev path on this user's machine).
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let scoop = PathBuf::from(home).join("scoop").join("apps").join("lazarus").join("current");
        if scoop.is_dir() { return Some(scoop) }
    }

    // Standard install paths.
    let candidates = if cfg!(target_os = "windows") {
        vec![
            PathBuf::from("C:/lazarus"),
            PathBuf::from("C:/Program Files/Lazarus"),
            PathBuf::from("C:/Program Files (x86)/Lazarus"),
            PathBuf::from("C:/fpcupdeluxe/lazarus"),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/usr/local/share/lazarus"),
            PathBuf::from("/Applications/Lazarus"),
        ]
    } else {
        vec![
            PathBuf::from("/usr/lib/lazarus"),
            PathBuf::from("/usr/share/lazarus"),
            PathBuf::from("/opt/lazarus"),
        ]
    };
    candidates.into_iter().find(|p| p.is_dir())
}

/// Enumerate `packages_dir` and push one ExternalDepRoot per package whose
/// `/src/` subdirectory exists. Platform-specific packages that are large and
/// irrelevant on the current host are skipped to avoid indexing thousands of
/// files that the project cannot use.
pub(crate) fn emit_package_roots(packages_dir: &Path, roots: &mut Vec<ExternalDepRoot>) {
    let Ok(entries) = std::fs::read_dir(packages_dir) else {
        return;
    };
    let mut pkg_names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    pkg_names.sort();

    for name in &pkg_names {
        // Skip packages whose platform does not match the current host.
        if is_platform_excluded(name) {
            continue;
        }
        let src = packages_dir.join(name).join("src");
        if !src.is_dir() {
            continue;
        }
        roots.push(ExternalDepRoot {
            module_path: format!("fpc-pkg-{name}"),
            version: String::new(),
            root: src,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }
}

/// Returns true when a package is large, platform-specific, and irrelevant
/// on the current host. Cross-platform packages always return false.
pub(crate) fn is_platform_excluded(pkg_name: &str) -> bool {
    // Cocoa / macOS bindings: only useful on macOS.
    if matches!(pkg_name, "cocoaint" | "iosxlocale" | "objcrtl" | "univint") {
        return !cfg!(target_os = "macos");
    }
    // X11 / GTK bindings: only useful on Linux/BSD.
    if matches!(pkg_name, "x11" | "gtk1" | "gtk2" | "fpgtk" | "gnome1" | "ggi" | "svgalib" | "ptc") {
        return !cfg!(target_os = "linux") && !cfg!(target_os = "freebsd");
    }
    // Win32 / Win CE bindings: only useful on Windows.
    if matches!(pkg_name, "winunits-base" | "winunits-jedi" | "winceunits") {
        return !cfg!(target_os = "windows");
    }
    // AROS / AmigaOS / MorphOS / Palm / DOS units: never relevant on a modern host.
    if matches!(pkg_name, "arosunits" | "ami-extra" | "amunits" | "os2units" | "os4units"
        | "morphunits" | "tosunits" | "palmunits" | "libgbafpc" | "libndsfpc" | "libogcfpc") {
        return true;
    }
    false
}
