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
//
// Demand-driven parsing
// ---------------------
// `uses_demand_driven_parse` is true: the eager walk is suppressed.
// `build_symbol_index` scans each `.pas`/`.pp` file for the unit name
// (first `unit <Name>;` declaration) and interface-section top-level
// declarations (`type`, `procedure`, `function`, `var`, `const` followed
// by an identifier), so the demand loop can locate the right source file
// for any runtime symbol without parsing the entire stdlib up front.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, SymbolLocationIndex,
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
        // A runtime locator owns no project-side caches. This set feeds the
        // PROJECT workspace scan — content dirs (tests/, examples/) must
        // never appear here or they prune every project's same-named dirs.
        &[]
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_freepascal_roots()
    }

    // Demand-driven: no eager walk. `build_symbol_index` registers each
    // Pascal unit's name and its interface-section declarations so the
    // Stage 2 loop can pull exactly the files a project's `uses` clause
    // references, without parsing the full ~900-file Lazarus+FPC stdlib.
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_pascal_symbol_index(dep_roots)
    }
}

impl ExternalSourceLocator for FreePascalRuntimeEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_freepascal_roots()
    }
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
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

/// Enumerate `packages_dir` and push one ExternalDepRoot per package whose
/// `/src/` subdirectory exists. Platform-specific packages that are large and
/// irrelevant on the current host are skipped to avoid indexing thousands of
/// files that the project cannot use.
fn emit_package_roots(packages_dir: &Path, roots: &mut Vec<ExternalDepRoot>) {
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
fn is_platform_excluded(pkg_name: &str) -> bool {
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

// ---------------------------------------------------------------------------
// Symbol index — cheap line-scan offering for demand-driven resolution
// ---------------------------------------------------------------------------

/// Build a `(module_path, name) → file` index over every Pascal source file
/// in `dep_roots` without a full tree-sitter parse. Two name shapes are
/// registered per file:
///
/// 1. The **unit name** extracted from the `unit <Name>;` declaration at the
///    top of each `.pas`/`.pp` — this is what a `uses SysUtils;` clause in
///    project code resolves against. Registered under both the unit name and
///    a lower-cased copy (Pascal identifiers are case-insensitive).
///
/// 2. Top-level declarations in the **interface section**: identifiers
///    following `type`, `procedure`, `function`, `var`, `const`, and `class`
///    keywords on their own line. These are the bare names a project uses
///    after a `uses SysUtils` (`Copy`, `Format`, `TStringList`, ...).
///
/// The scan reads each file line by line and stops at `implementation` so it
/// never descends into function bodies — keeping the scan O(interface size)
/// rather than O(file size).
fn build_pascal_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut idx = SymbolLocationIndex::new();
    for dep in dep_roots {
        collect_pascal_names_rec(&dep.root, &dep.root, dep, &mut idx, 0);
    }
    if !idx.is_empty() {
        debug!("freepascal: indexed {} Pascal symbol locations", idx.len());
    }
    idx
}

fn collect_pascal_names_rec(
    root: &Path,
    dir: &Path,
    dep: &ExternalDepRoot,
    idx: &mut SymbolLocationIndex,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "examples" | "demos" | "languages" | "images") {
                    continue;
                }
                if name.starts_with('.') { continue }
            }
            collect_pascal_names_rec(root, &path, dep, idx, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let lower = name.to_ascii_lowercase();
            if !lower.ends_with(".pas") && !lower.ends_with(".pp") {
                // .inc and .lpr files rarely declare unit-level names and
                // are included by the host .pas; skip to keep the index lean.
                continue;
            }
            scan_pascal_file(&path, dep, idx);
        }
    }
}

/// Scan a single Pascal source file and register all unit-level names in `idx`.
///
/// Keyword matching is case-insensitive (Pascal convention). Identifiers are
/// registered under their as-declared form AND their lowercase form so
/// callers need not know the declaration casing.
fn scan_pascal_file(path: &Path, dep: &ExternalDepRoot, idx: &mut SymbolLocationIndex) {
    let Ok(content) = std::fs::read_to_string(path) else { return };
    let module = &dep.module_path;

    let mut in_interface = false;
    let mut past_unit_decl = false;
    // When the previous interface line was a section keyword on its own
    // (`type`, `var`, `const`), record which keyword so the next
    // identifier-only line is treated as a declaration in that section.
    let mut pending_section: Option<&str> = None;

    for raw_line in content.lines() {
        let stripped = strip_pascal_line_comment(raw_line).trim();
        if stripped.is_empty() { continue }
        // Lower-case copy for keyword matching; the original `stripped` slice
        // preserves case for identifier registration.
        let lower = stripped.to_ascii_lowercase();

        // Extract the unit name from the `unit <Name>;` header.
        if !past_unit_decl {
            if let Some(rest) = lower.strip_prefix("unit ") {
                // Extract the original-case unit name by slicing `stripped`.
                let original_rest = &stripped["unit ".len()..];
                let unit_name = original_rest.trim_end_matches(';').trim();
                if !unit_name.is_empty() && is_pascal_ident(unit_name) {
                    idx.insert(module, unit_name, path);
                    let lc = unit_name.to_ascii_lowercase();
                    if lc != unit_name { idx.insert(module, &lc, path); }
                }
                // Consume the rest variable to avoid an unused-variable warning.
                let _ = rest;
                past_unit_decl = true;
            }
            continue;
        }

        if lower == "interface" {
            in_interface = true;
            pending_section = None;
            continue;
        }
        if lower == "implementation" {
            break;
        }

        if !in_interface { continue }

        // Detect bare section keywords (`type`, `var`, `const`) on their own
        // line — common Pascal style for a block of declarations. A line is
        // "bare" when the keyword is the entire content (no following ident).
        let is_bare_section = matches!(lower.as_str(), "type" | "var" | "const");
        if is_bare_section {
            pending_section = Some(match lower.as_str() {
                "type" => "type ",
                "var" => "var ",
                _ => "const ",
            });
            continue;
        }

        // A line that starts with `procedure`, `function`, or `class` (with a
        // space following, meaning it has an ident on the same line) belongs to
        // the top-level interface — clear any pending section context so these
        // are parsed via `extract_decl_ident` rather than the bare-ident path.
        let starts_new_decl = lower.starts_with("procedure ")
            || lower.starts_with("function ")
            || lower.starts_with("class ");
        if starts_new_decl {
            pending_section = None;
        }

        // Try to extract a declared identifier. Both branches recover the
        // original-case spelling from `stripped` using the byte offset
        // determined from the lowercase `lower` slice (byte lengths are
        // identical for ASCII identifiers).
        let ident: Option<&str> = if let Some(_section) = pending_section {
            // Line directly follows a bare section keyword (`type`, `var`,
            // `const`). The identifier starts at position 0 of the line.
            let end = lower
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(lower.len());
            let name_lower = &lower[..end];
            if !name_lower.is_empty() && is_pascal_ident(name_lower) {
                Some(&stripped[..end])
            } else {
                None
            }
        } else {
            extract_decl_ident(&lower).map(|found| {
                let offset = found.as_ptr() as usize - lower.as_ptr() as usize;
                &stripped[offset..offset + found.len()]
            })
        };

        if let Some(name) = ident {
            if !name.is_empty() && is_pascal_ident(name) {
                idx.insert(module, name, path);
                let lc = name.to_ascii_lowercase();
                if lc != name { idx.insert(module, &lc, path); }
            }
        }
    }
}

/// Return the declared identifier from a Pascal interface-section
/// declaration line, or `None` if the line doesn't match a recognized pattern.
///
/// Recognised forms (case-insensitive):
///   - `procedure Foo` / `procedure Foo(...)` → `"Foo"`
///   - `function Bar(...)` → `"Bar"`
///   - `type TMyClass` / `type TMyClass = ...` → `"TMyClass"`
///   - `var FField: TType` → `"FField"`
///   - `const MAX_SIZE = ...` → `"MAX_SIZE"`
///   - `class TFoo` → `"TFoo"`
fn extract_decl_ident(line: &str) -> Option<&str> {
    for kw in &["procedure ", "function ", "type ", "var ", "const ", "class "] {
        if let Some(rest) = line.strip_prefix(kw) {
            let rest = rest.trim_start();
            // Take the identifier up to the first non-identifier character.
            let end = rest
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            let name = &rest[..end];
            if !name.is_empty() && is_pascal_ident(name) {
                return Some(name);
            }
        }
    }
    None
}

/// True when `s` is a valid Pascal identifier: starts with a letter or
/// underscore, followed by letters, digits, or underscores.
fn is_pascal_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return false };
    if !first.is_alphabetic() && first != '_' { return false }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Remove a `//` or `{...}` line comment from `line`, returning the
/// text before the comment. Block comments `{...}` spanning a single line
/// are stripped; multi-line `{...}` blocks are not tracked here — they're
/// uncommon in interface-section declaration lines and treated as noise.
fn strip_pascal_line_comment(line: &str) -> &str {
    // Slash-slash line comment.
    if let Some(idx) = line.find("//") {
        return &line[..idx];
    }
    // Single-line brace comment: `{ ... }`.
    if let Some(open) = line.find('{') {
        if let Some(close) = line[open..].find('}') {
            let _ = close; // close position within the slice
            // Return everything before the `{`.
            return &line[..open];
        }
    }
    line
}

// ---------------------------------------------------------------------------
// Test handles
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(super) fn _test_scan_pascal_file(
    path: &Path,
    dep: &ExternalDepRoot,
    idx: &mut SymbolLocationIndex,
) {
    scan_pascal_file(path, dep, idx);
}

#[cfg(test)]
pub(super) fn _test_build_pascal_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    build_pascal_symbol_index(dep_roots)
}

#[cfg(test)]
pub(super) fn _test_extract_decl_ident(line: &str) -> Option<&str> {
    extract_decl_ident(line)
}

#[cfg(test)]
#[path = "freepascal_runtime_tests.rs"]
mod tests;
