// =============================================================================
// ecosystem/r_stdlib.rs — R base / recommended packages (stdlib ecosystem)
//
// R's base packages (`base`, `stats`, `utils`, `graphics`, `methods`,
// `tools`, `datasets`) are bundled with the R install BUT shipped in
// compressed binary lazy-load format under
// `<R_HOME>/library/<pkg>/R/<pkg>` (the `.rdb`/`.rdx` pair). Those are
// not walkable as source — there is no plain-text `.R` to feed the R
// extractor.
//
// Two discovery paths are attempted in order:
//
//   1. **Source distribution** — plain-text `.R` files under
//      `<R-src>/src/library/<pkg>/R/*.R`. Only available when the user
//      has an R source checkout and sets `BEARWISDOM_R_SRC`. Each file
//      becomes a `WalkedFile` and is parsed by the R extractor as normal.
//
//   2. **Installed R** — every R install ships plain-text `NAMESPACE` files
//      at `<R_HOME>/library/<pkg>/NAMESPACE` listing exported symbols.
//      Probed via `R_HOME` env var, the Windows registry
//      (`HKLM\Software\R-core\R` / `R64`), and `C:\Program Files\R\R-*`
//      directory enumeration. On Unix, `R RHOME` subprocess output is
//      used as a last resort. Symbols are synthesized from NAMESPACE
//      directives and returned via `parse_metadata_only` — no tree-sitter
//      parse needed.
//
// When neither source nor an install is found the walker emits nothing,
// consistent with the trait doc's "missing toolchain" degrade-honestly
// behaviour.
//
// Activation: `LanguagePresent("r")` — every R project uses these names.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("r-stdlib");
const TAG: &str = "r-stdlib";
const LANGUAGES: &[&str] = &["r"];

/// Discriminator embedded in `module_path` to distinguish the two walk modes.
const KIND_SOURCE: &str = "r-stdlib";
const KIND_NAMESPACE: &str = "r-stdlib-ns";

/// R base packages shipped in `<R-src>/src/library/` or `<R_HOME>/library/`.
const BASE_PACKAGES: &[&str] = &[
    "base", "stats", "utils", "graphics", "grDevices", "methods",
    "tools", "datasets", "stats4", "splines", "grid", "parallel",
    "compiler", "tcltk",
];

pub struct RStdlibEcosystem;

impl Ecosystem for RStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("r")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_r_stdlib()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        if dep.module_path == KIND_SOURCE {
            walk_r_tree(dep)
        } else {
            Vec::new()
        }
    }

    fn parse_metadata_only(&self, dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        if dep.module_path == KIND_NAMESPACE {
            Some(synthesize_from_namespace(&dep.root))
        } else {
            None
        }
    }

    fn supports_reachability(&self) -> bool { true }
    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for RStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_r_stdlib()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        if dep.module_path == KIND_SOURCE {
            walk_r_tree(dep)
        } else {
            Vec::new()
        }
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        // Legacy locator interface — discover all installed libraries and
        // synthesize symbols from each.
        let mut out = Vec::new();
        for root in discover_r_stdlib() {
            if root.module_path == KIND_NAMESPACE {
                out.extend(synthesize_from_namespace(&root.root));
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<RStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(RStdlibEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

pub(super) fn discover_r_stdlib() -> Vec<ExternalDepRoot> {
    // Priority 1: explicit source distribution via BEARWISDOM_R_SRC.
    if let Some(root) = probe_r_source_distro() {
        return vec![root];
    }
    // Priority 2: installed R — yields NAMESPACE-based synthetic symbols.
    // Returns one root per library found (system + user libraries).
    let installed = probe_r_install();
    if !installed.is_empty() {
        return installed;
    }
    debug!("r-stdlib: no R source distribution or install found \
            (set BEARWISDOM_R_SRC or R_HOME, or install R)");
    Vec::new()
}

/// Probe an R source distribution (plain `.R` files).
fn probe_r_source_distro() -> Option<ExternalDepRoot> {
    let explicit = std::env::var_os("BEARWISDOM_R_SRC")?;
    let r_src = PathBuf::from(explicit);
    if !r_src.is_dir() {
        return None;
    }
    let library_root = r_src.join("src").join("library");
    if !library_root.is_dir() {
        debug!(
            "r-stdlib: BEARWISDOM_R_SRC={} does not contain src/library/",
            r_src.display()
        );
        return None;
    }
    debug!("r-stdlib: using source distro at {}", library_root.display());
    Some(ExternalDepRoot {
        module_path: KIND_SOURCE.to_string(),
        version: String::new(),
        root: library_root,
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    })
}

/// Probe an installed R instance. Returns one `ExternalDepRoot` per library
/// root found: the system library at `<R_HOME>/library`, plus any user
/// libraries from `R_LIBS_USER` env var or the platform-default user-library
/// path. Each library contains one subdirectory per installed package.
fn probe_r_install() -> Vec<ExternalDepRoot> {
    let mut out = Vec::new();
    if let Some(r_home) = find_r_home() {
        let library = r_home.join("library");
        // base lacks NAMESPACE (built into R itself); DESCRIPTION is the
        // load-bearing per-package metadata file present on every install.
        if library.join("base").join("DESCRIPTION").is_file() {
            debug!("r-stdlib: using system library at {}", library.display());
            out.push(ExternalDepRoot {
                module_path: KIND_NAMESPACE.to_string(),
                version: String::new(),
                root: library,
                ecosystem: TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
        } else {
            debug!(
                "r-stdlib: {} does not look like a real R install (missing library/base/DESCRIPTION)",
                r_home.display()
            );
        }
    }
    for user_lib in find_user_libraries() {
        debug!("r-stdlib: using user library at {}", user_lib.display());
        out.push(ExternalDepRoot {
            module_path: KIND_NAMESPACE.to_string(),
            version: String::new(),
            root: user_lib,
            ecosystem: TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }
    out
}

/// Locate user-installed R package libraries. Returns every existing path
/// found via `R_LIBS_USER` env var, plus platform-default user-library
/// directories (the locations `install.packages()` writes to by default).
fn find_user_libraries() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(val) = std::env::var_os("R_LIBS_USER") {
        for piece in std::env::split_paths(&val) {
            collect_library_versions(&piece, &mut out);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(home) = std::env::var_os("USERPROFILE") {
            let home = PathBuf::from(home);
            collect_library_versions(&home.join("Documents").join("R").join("win-library"), &mut out);
            collect_library_versions(&home.join("AppData").join("Local").join("R").join("win-library"), &mut out);
            collect_library_versions(&home.join("R").join("win-library"), &mut out);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            collect_library_versions(&PathBuf::from(home).join("R"), &mut out);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// User library roots are typically organized one subdirectory per
/// R version (`win-library/4.6`, `win-library/4.5`, …). Add each
/// existing version-subdirectory. If the path itself directly contains
/// packages (no version subdirs), add it directly.
fn collect_library_versions(root: &Path, out: &mut Vec<PathBuf>) {
    if !root.is_dir() {
        return;
    }
    let mut found_subdir = false;
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && looks_like_library_dir(&path) {
                out.push(path);
                found_subdir = true;
            }
        }
    }
    if !found_subdir && looks_like_library_dir(root) {
        out.push(root.to_path_buf());
    }
}

/// A library directory contains one subdirectory per installed package,
/// each of which has a `DESCRIPTION` file. Spot-check by looking for any
/// child directory with a DESCRIPTION.
fn looks_like_library_dir(path: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else { return false };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() && p.join("DESCRIPTION").is_file() {
            return true;
        }
    }
    false
}

/// Locate `R_HOME` by trying, in order:
///   1. `R_HOME` env var
///   2. Windows registry (`HKLM\Software\R-core\R64`, then `\R`)
///   3. `C:\Program Files\R\R-*` directory enumeration (newest version wins)
///   4. `R RHOME` subprocess (Unix / when R is on PATH)
fn find_r_home() -> Option<PathBuf> {
    // 1. Explicit env var.
    if let Some(val) = std::env::var_os("R_HOME") {
        let p = PathBuf::from(val);
        if p.is_dir() {
            return Some(p);
        }
    }

    // 2. Windows registry.
    #[cfg(target_os = "windows")]
    if let Some(p) = probe_windows_registry() {
        return Some(p);
    }

    // 3. Windows filesystem probe.
    #[cfg(target_os = "windows")]
    if let Some(p) = probe_windows_program_files() {
        return Some(p);
    }

    // 4. `R RHOME` subprocess (Unix; also works on Windows with R on PATH).
    probe_r_rhome_subprocess()
}

/// Read `InstallPath` from `HKLM\Software\R-core\R64` then `\R` using
/// the `reg query` subprocess — no additional crate dependency required.
#[cfg(target_os = "windows")]
fn probe_windows_registry() -> Option<PathBuf> {
    for subkey in &[
        r"HKLM\Software\R-core\R64",
        r"HKLM\Software\R-core\R",
    ] {
        if let Some(p) = read_registry_install_path(subkey) {
            return Some(p);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn read_registry_install_path(subkey: &str) -> Option<PathBuf> {
    let output = std::process::Command::new("reg")
        .args(["query", subkey, "/v", "InstallPath"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    // `reg query` output looks like:
    //   HKLM\Software\R-core\R64
    //       InstallPath    REG_SZ    C:\Program Files\R\R-4.4.0
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("InstallPath") {
            // After "InstallPath" comes whitespace + "REG_SZ" + whitespace + path.
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2 {
                let p = PathBuf::from(parts[1..].join(" "));
                if p.is_dir() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Enumerate `C:\Program Files\R\R-*` directories and return the newest version.
#[cfg(target_os = "windows")]
fn probe_windows_program_files() -> Option<PathBuf> {
    let base = PathBuf::from(r"C:\Program Files\R");
    if !base.is_dir() {
        return None;
    }
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&base)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let ft = e.file_type().ok()?;
            if !ft.is_dir() { return None; }
            let name = e.file_name();
            let s = name.to_string_lossy();
            // Match "R-4.3.2", "R-4.4.0-win", etc.
            if s.starts_with("R-") { Some(e.path()) } else { None }
        })
        .collect();

    // Sort lexicographically — "R-4.4.0" > "R-4.3.2" by string comparison,
    // which works for standard semver-like directory names.
    candidates.sort();
    candidates.into_iter().rev().next()
}

/// Shell out to `R RHOME` to get the R home directory. Works on Unix when `R`
/// is on PATH; also works on Windows if R is on PATH but not in the registry.
fn probe_r_rhome_subprocess() -> Option<PathBuf> {
    let output = std::process::Command::new("R")
        .args(["RHOME"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8(output.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    let p = PathBuf::from(trimmed);
    if p.is_dir() { Some(p) } else { None }
}

// ---------------------------------------------------------------------------
// Source-distro walk (existing path — unchanged)
// ---------------------------------------------------------------------------

fn walk_r_tree(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    for pkg in BASE_PACKAGES {
        let pkg_r_dir = dep.root.join(pkg).join("R");
        if !pkg_r_dir.is_dir() { continue }
        walk_dir(&pkg_r_dir, &mut out, 0);
    }
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= 6 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') { continue }
            }
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".R") && !name.ends_with(".r") { continue }
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:r-stdlib:{display}"),
                absolute_path: path,
                language: "r",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// NAMESPACE-based symbol synthesis (installed-R path)
// ---------------------------------------------------------------------------

/// Enumerate every package subdirectory under `library_root`, parse its
/// NAMESPACE file for exports, and return one `ParsedFile` containing all
/// exported symbols. Packages without a NAMESPACE (notably `base`, which
/// is built into R itself) are dumped via a single Rscript subprocess that
/// calls `getNamespaceExports()` — the authoritative R-side enumeration.
pub(super) fn synthesize_from_namespace(library_root: &Path) -> Vec<ParsedFile> {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut needs_rscript: Vec<String> = Vec::new();

    let Ok(entries) = std::fs::read_dir(library_root) else {
        debug!("r-stdlib: could not read library root {}", library_root.display());
        return Vec::new();
    };
    for entry in entries.flatten() {
        let pkg_dir = entry.path();
        if !pkg_dir.is_dir() {
            continue;
        }
        let Some(pkg) = pkg_dir.file_name().and_then(|n| n.to_str()) else { continue };
        if pkg.starts_with('.') {
            continue;
        }
        // Each real R package has a DESCRIPTION file; skip non-package directories.
        if !pkg_dir.join("DESCRIPTION").is_file() {
            continue;
        }

        let ns_path = pkg_dir.join("NAMESPACE");
        if ns_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&ns_path) {
                parse_namespace(&content, pkg, &mut symbols);
                continue;
            }
        }
        // Defer to the Rscript pass — the only authoritative source for
        // packages without a NAMESPACE file (base is hardcoded inside R).
        needs_rscript.push(pkg.to_string());
    }

    if !needs_rscript.is_empty() {
        // library_root is `<R_HOME>/library` — derive r_home for Rscript.
        if let Some(r_home) = library_root.parent() {
            let rscript_exports = dump_exports_via_rscript(r_home, &needs_rscript);
            for (pkg, names) in rscript_exports {
                for name in names {
                    if !name.is_empty() {
                        symbols.push(make_sym(&name, &pkg, SymbolKind::Function));
                    }
                }
            }
        }
    }

    if symbols.is_empty() {
        return Vec::new();
    }

    let n = symbols.len();
    vec![ParsedFile {
        path: "ext:r-stdlib:r_stdlib_generated.R".to_string(),
        language: "r".to_string(),
        content_hash: format!("r-stdlib-ns-{n}"),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None; n],
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: vec![false; n],
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }]
}

/// Parse one NAMESPACE file and append `ExtractedSymbol` entries to `out`.
///
/// Directives handled:
///   `export(f1, f2, ...)` — exported functions/objects
///   `export(`backtick name`)` — names with special characters
///   `S3method(generic, class)` — S3 dispatch registrations
///   `exportPattern(regex)` — ignored (can't enumerate without environment)
///   `exportClasses(Cl1, ...)` — S4 class exports
///   `exportMethods(f1, ...)` — S4 generic method exports
///   `exportClassesFrom(pkg, ...)` — re-exports; emitted as Class symbols
///
/// NAMESPACE directives commonly span multiple lines — `export(\n  a,\n  b,\n)` —
/// so the parser walks the content collecting balanced-paren directive bodies
/// before dispatching, not line-by-line.
pub(super) fn parse_namespace(content: &str, pkg: &str, out: &mut Vec<ExtractedSymbol>) {
    for directive in iter_directives(content) {
        let trimmed = directive.trim_start();
        if let Some(rest) = strip_directive(trimmed, "export") {
            extract_names(rest, pkg, SymbolKind::Function, out);
        } else if let Some(rest) = strip_directive(trimmed, "exportClasses") {
            extract_names(rest, pkg, SymbolKind::Class, out);
        } else if let Some(rest) = strip_directive(trimmed, "exportMethods") {
            extract_names(rest, pkg, SymbolKind::Function, out);
        } else if let Some(rest) = strip_directive(trimmed, "exportClassesFrom") {
            // First argument is the source package; remaining are class names.
            // Emit the class names only (source package is informational).
            if let Some(inner) = extract_paren_body(rest) {
                let mut args = split_args(inner);
                if args.len() > 1 {
                    args.remove(0);
                    for name in args {
                        let name = clean_name(name);
                        if !name.is_empty() {
                            out.push(make_sym(&name, pkg, SymbolKind::Class));
                        }
                    }
                }
            }
        } else if let Some(rest) = strip_directive(trimmed, "S3method") {
            // S3method(generic, class) — the dispatch name seen at call sites
            // is `generic`, so emit the generic as a function symbol.
            if let Some(inner) = extract_paren_body(rest) {
                let args = split_args(inner);
                if let Some(generic) = args.first() {
                    let name = clean_name(generic);
                    if !name.is_empty() {
                        out.push(make_sym(&name, pkg, SymbolKind::Function));
                    }
                }
            }
        }
        // exportPattern, useDynLib, import, importFrom etc. are intentionally
        // skipped: they either can't enumerate names without an R environment
        // or describe inbound rather than outbound symbols.
    }
}

/// Dump per-package exports via a single Rscript subprocess call. Returns
/// a list of `(package, exports)` pairs in the order requested. Skipped
/// silently if Rscript can't be located or invoked — fallback path is
/// "this package has no exports we can see," consistent with how the
/// other Stdlib walkers degrade.
///
/// The script delimits per-package output with a sentinel line so a single
/// subprocess call covers every package in one round trip.
pub(super) fn dump_exports_via_rscript(
    r_home: &Path,
    packages: &[String],
) -> Vec<(String, Vec<String>)> {
    let rscript = if cfg!(target_os = "windows") {
        r_home.join("bin").join("Rscript.exe")
    } else {
        r_home.join("bin").join("Rscript")
    };
    let rscript_bin = if rscript.is_file() {
        rscript
    } else {
        // Fallback to PATH.
        PathBuf::from(if cfg!(target_os = "windows") { "Rscript.exe" } else { "Rscript" })
    };

    // Build a single R expression that prints each package's exports
    // surrounded by sentinel markers.
    let pkgs_arg = packages
        .iter()
        .map(|p| format!("'{}'", p.replace('\'', "")))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "for (p in c({pkgs_arg})) {{ \
           cat('---R-STDLIB-PKG:', p, '---\\n', sep=''); \
           tryCatch(cat(getNamespaceExports(p), sep='\\n'), error=function(e) NULL); \
           cat('\\n'); \
         }}"
    );

    let output = match std::process::Command::new(&rscript_bin)
        .args(["-e", &script])
        .output()
    {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            debug!(
                "r-stdlib: Rscript exit {} for getNamespaceExports — falling back to NAMESPACE-only coverage",
                o.status.code().unwrap_or(-1)
            );
            return Vec::new();
        }
        Err(e) => {
            debug!("r-stdlib: could not invoke Rscript: {e}");
            return Vec::new();
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("---R-STDLIB-PKG:") {
            if let Some(done) = current.take() {
                out.push(done);
            }
            let pkg = rest.trim_end_matches("---").trim().to_string();
            current = Some((pkg, Vec::new()));
        } else if let Some((_, names)) = current.as_mut() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                names.push(trimmed.to_string());
            }
        }
    }
    if let Some(done) = current.take() {
        out.push(done);
    }
    out
}

/// Walk the NAMESPACE content yielding one logical directive per call,
/// joining lines until parens balance. Comments (`# ...`) are stripped per
/// line. Backtick-quoted segments don't count for paren-depth.
pub(super) fn iter_directives(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut depth: i32 = 0;
    let mut in_backtick = false;
    for raw_line in content.lines() {
        // Strip comments — but only when NOT inside a backtick segment.
        let line = if in_backtick {
            raw_line
        } else {
            match raw_line.find('#') {
                Some(idx) => &raw_line[..idx],
                None => raw_line,
            }
        };
        for ch in line.chars() {
            match ch {
                '`' => in_backtick = !in_backtick,
                '(' if !in_backtick => depth += 1,
                ')' if !in_backtick => depth -= 1,
                _ => {}
            }
            buf.push(ch);
        }
        buf.push('\n');
        if depth <= 0 {
            let directive = std::mem::take(&mut buf);
            let t = directive.trim();
            if !t.is_empty() {
                out.push(directive);
            }
            depth = 0;
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf);
    }
    out
}

/// Strip a directive name and return the remainder (including the `(`).
/// Returns `None` when the line doesn't start with `<directive>(`.
fn strip_directive<'a>(line: &'a str, directive: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(directive)?;
    // Allow optional whitespace before `(`.
    let rest = rest.trim_start();
    if rest.starts_with('(') { Some(rest) } else { None }
}

/// Extract the text between the outermost `(` and `)`. Handles multi-line
/// NAMESPACE entries by stopping at the first `)` that closes the opening `(`.
fn extract_paren_body(s: &str) -> Option<&str> {
    let start = s.find('(')?;
    let end = s.rfind(')')?;
    if end > start { Some(&s[start + 1..end]) } else { None }
}

/// Split a comma-separated argument list, respecting backtick-quoted names
/// and ignoring interior parentheses.
fn split_args(s: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut depth = 0usize;
    let mut in_backtick = false;
    let mut start = 0;

    for (i, ch) in s.char_indices() {
        match ch {
            '`' => in_backtick = !in_backtick,
            '(' if !in_backtick => depth += 1,
            ')' if !in_backtick => depth = depth.saturating_sub(1),
            ',' if !in_backtick && depth == 0 => {
                args.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        args.push(&s[start..]);
    }
    args
}

/// Strip backtick quoting, leading/trailing whitespace, and surrounding quotes.
fn clean_name(s: &str) -> String {
    let s = s.trim();
    // Backtick-quoted: `foo.bar`
    if s.starts_with('`') && s.ends_with('`') && s.len() >= 2 {
        return s[1..s.len() - 1].to_string();
    }
    // Double-quoted: "foo"
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}

/// Parse a directive whose argument list is a sequence of exported names and
/// append one `ExtractedSymbol` per name.
fn extract_names(
    rest: &str,
    pkg: &str,
    kind: SymbolKind,
    out: &mut Vec<ExtractedSymbol>,
) {
    let Some(inner) = extract_paren_body(rest) else { return };
    for raw in split_args(inner) {
        let name = clean_name(raw);
        if !name.is_empty() {
            out.push(make_sym(&name, pkg, kind));
        }
    }
}

fn make_sym(name: &str, pkg: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: format!("{pkg}::{name}"),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("# {pkg} NAMESPACE export")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
    }
}

#[cfg(test)]
#[path = "r_stdlib_tests.rs"]
mod tests;
