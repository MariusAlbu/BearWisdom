// =============================================================================
// ecosystem/freepascal_runtime.rs — Lazarus IDE + Free Pascal stdlib
//
// Probes a Lazarus install and surfaces three on-disk source trees as
// external dep roots:
//
//   - <root>/lcl/         — LCL (`TForm`, `TButton`, `TMenuItem`, etc.)
//   - <root>/components/  — Lazarus-bundled components (`codetools`, `chmhelp`)
//   - <root>/fpc/<ver>/source/{rtl,packages}/ — Free Pascal RTL + FCL
//                                              (`SysUtils`, `Classes`, `Math`)
//
// Activation: any Pascal project (`.pas`/`.pp`/`.lpr`/`.lpi`/`.lpk`).
// Probes scoop/apps/lazarus/current/ first (Windows scoop convention),
// then $LAZARUS_DIR, then standard install paths on each platform.
// Sources are walked in place — no extraction step (Pascal sources ship
// directly on disk, unlike JVM jars).
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("freepascal-runtime");
const LEGACY_ECOSYSTEM_TAG: &str = "freepascal-runtime";
const LANGUAGES: &[&str] = &["pascal"];

pub struct FreePascalRuntimeEcosystem;

impl Ecosystem for FreePascalRuntimeEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("pascal")
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        // Lazarus ships test fixtures and example apps inside the LCL
        // package tree. Skip them so they don't leak into project symbols.
        &["tests", "examples", "demos", "ide", "designer"]
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_freepascal_roots()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_pascal_root(dep)
    }

    // Eager walk — FPC stdlib is small enough and demand-driven would
    // require a build_symbol_index impl that pre-parses every .pas/.pp
    // file to populate name → file map, which is essentially the same
    // cost as parsing them up front.
}

impl ExternalSourceLocator for FreePascalRuntimeEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_freepascal_roots()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_pascal_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<FreePascalRuntimeEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(FreePascalRuntimeEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_freepascal_roots() -> Vec<ExternalDepRoot> {
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
                    roots.push(ExternalDepRoot {
                        module_path: format!("fpc-rtl-{target}"),
                        version: String::new(),
                        root: candidate,
                        ecosystem: LEGACY_ECOSYSTEM_TAG,
                        package_id: None,
                        requested_imports: Vec::new(),
                    });
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
            // FCL packages (XML, registry, fpvectorial, ...).
            let packages = source.join("packages");
            if packages.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: "fpc-packages".to_string(),
                    version: String::new(),
                    root: packages,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
            }
        }
    }

    debug!("FreePascal runtime: {} roots", roots.len());
    roots
}

/// Return the host platform's expected FPC RTL subdirectory name(s).
/// Listed in fallback order — first match wins.
fn rtl_host_targets() -> &'static [&'static str] {
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

fn first_subdir(dir: &Path) -> Option<PathBuf> {
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

fn lazarus_install_root() -> Option<PathBuf> {
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

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_pascal_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_pascal_dir(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_pascal_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "examples" | "demos" | "languages" | "images") {
                    continue;
                }
                if name.starts_with('.') { continue }
            }
            walk_pascal_dir(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !is_pascal_source(name) { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:pascal:{}/{rel_sub}", dep.module_path);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "pascal",
            });
        }
    }
}

fn is_pascal_source(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".pas")
        || lower.ends_with(".pp")
        || lower.ends_with(".lpr")
        || lower.ends_with(".inc")
        || lower.ends_with(".dpr")
}

#[cfg(test)]
#[path = "freepascal_runtime_tests.rs"]
mod tests;
