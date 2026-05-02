// =============================================================================
// ecosystem/prolog_runtime.rs — on-disk discovery of SWI-Prolog runtime source
//
// Real Prolog projects use the SWI-Prolog standard library (`library(lists)`,
// `library(apply)`, `library(http/http_dispatch)`, ...) plus the autoloaded
// boot-time predicates from `boot/` (`format`, `assertz`, `findall`, etc.).
// None of those predicate names are user-defined inside the project; they
// live in the SWI-Prolog source tree shipped on the user's machine.
//
// **Discovery strategy** (no vendored data, real on-disk only):
//   1. `BEARWISDOM_SWIPL_SOURCE` env var — explicit path to a checkout of
//      `github.com/SWI-Prolog/swipl-devel` or an installed SWI-Prolog runtime
//      whose layout exposes `library/` and `boot/`.
//   2. Common dev-machine clone locations: `~/repos/swipl-devel`,
//      `~/source/swipl-devel`, `~/work/swipl-devel`, `~/code/swipl-devel`,
//      `~/src/swipl-devel`.
//   3. Common install layouts:
//        * Windows: `C:\Program Files\swipl`, `C:\Program Files (x86)\swipl`.
//        * Linux:   `/usr/lib/swi-prolog`, `/usr/local/lib/swi-prolog`,
//                   `/opt/swi-prolog`.
//        * macOS:   `/opt/homebrew/Cellar/swi-prolog/*/libexec/lib/swipl`,
//                   `/usr/local/Cellar/swi-prolog/*/libexec/lib/swipl`.
//   4. Ask the installed `swipl` binary directly:
//      `swipl --dump-runtime-variables` exposes `PLLIBDIR=`.
//
// When discovery succeeds we register `library/` and `boot/` as
// ExternalDepRoots. The standard Prolog plugin walks `.pl` files in each
// root and emits Function symbols for predicate definitions — exactly the
// shape `lookup.by_name(target)` needs.
//
// **When discovery fails** (no clone, no install, no env var): refs to
// stdlib predicates stay unresolved. That's the honest signal — the names
// are genuinely unindexable from this machine.
//
// Activation: any `.pl` / `.pro` / `.P` file present in the project.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("prolog-runtime");
const ECOSYSTEM_TAG: &str = "prolog-runtime";
const LANGUAGES: &[&str] = &["prolog"];

pub struct PrologRuntimeEcosystem;

impl Ecosystem for PrologRuntimeEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("prolog")
    }

    fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_swipl_roots()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_prolog_tree(dep)
    }

    // Eager walk: SWI-Prolog's library/ + boot/ together hold a few hundred
    // .pl files (~3 MB). Walking eagerly keeps the build_symbol_index
    // requirement out of scope. Demand-driven parsing was a copy-paste
    // from the bicep_runtime template that doesn't fit the .pl walk
    // shape.
    fn uses_demand_driven_parse(&self) -> bool { false }
}

impl ExternalSourceLocator for PrologRuntimeEcosystem {
    fn ecosystem(&self) -> &'static str { ECOSYSTEM_TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_swipl_roots()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_prolog_tree(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PrologRuntimeEcosystem>> = OnceLock::new();
    LOCATOR
        .get_or_init(|| Arc::new(PrologRuntimeEcosystem))
        .clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_swipl_roots() -> Vec<ExternalDepRoot> {
    let Some(root) = find_swipl_source() else { return Vec::new() };
    let mut roots: Vec<ExternalDepRoot> = Vec::new();

    let library = root.join("library");
    if library.is_dir() {
        roots.push(ExternalDepRoot {
            module_path: "swipl/library".to_string(),
            version: String::from("local"),
            root: library,
            ecosystem: ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }
    let boot = root.join("boot");
    if boot.is_dir() {
        roots.push(ExternalDepRoot {
            module_path: "swipl/boot".to_string(),
            version: String::from("local"),
            root: boot,
            ecosystem: ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }

    if !roots.is_empty() {
        tracing::info!(
            "prolog-runtime: using SWI-Prolog source at {}",
            root.display()
        );
    }
    roots
}

/// Resolve a SWI-Prolog source tree on disk. Returns the directory whose
/// children include `library/` (and ideally `boot/`).
pub(crate) fn find_swipl_source() -> Option<PathBuf> {
    // 1. Explicit override.
    if let Some(raw) = std::env::var_os("BEARWISDOM_SWIPL_SOURCE") {
        let p = PathBuf::from(raw);
        if looks_like_swipl_source(&p) {
            return Some(p);
        }
    }

    // 2. Common dev-checkout locations under $HOME / $USERPROFILE.
    if let Some(home) = home_dir() {
        for sub in &[
            "repos/swipl-devel",
            "source/swipl-devel",
            "work/swipl-devel",
            "code/swipl-devel",
            "src/swipl-devel",
            "repos/swipl",
            "source/swipl",
        ] {
            let candidate = home.join(sub);
            if looks_like_swipl_source(&candidate) {
                return Some(candidate);
            }
        }
    }

    // 3. Standard install layouts. SWI-Prolog ships its `.pl` library
    // alongside the binary on every platform — these are real source files,
    // not vendored copies, and parsing them is the canonical resolution
    // path for any Prolog project on the machine.
    let install_candidates: &[&str] = if cfg!(target_os = "windows") {
        &[
            "C:/Program Files/swipl",
            "C:/Program Files (x86)/swipl",
        ]
    } else if cfg!(target_os = "macos") {
        &[
            "/opt/homebrew/lib/swipl",
            "/usr/local/lib/swipl",
            "/opt/local/lib/swipl",
        ]
    } else {
        &[
            "/usr/lib/swi-prolog",
            "/usr/local/lib/swi-prolog",
            "/usr/lib/swipl",
            "/usr/local/lib/swipl",
            "/opt/swi-prolog",
            "/opt/swipl",
        ]
    };
    for candidate in install_candidates {
        let p = PathBuf::from(candidate);
        if looks_like_swipl_source(&p) {
            return Some(p);
        }
    }

    // 4. Ask the binary directly: `swipl --dump-runtime-variables` exposes
    // `PLLIBDIR=...` — that's the directory whose contents are the .pl
    // library. We want its parent so `library/` and `boot/` resolve.
    if let Ok(output) = Command::new("swipl")
        .args(["--dump-runtime-variables"])
        .output()
    {
        if output.status.success() {
            if let Ok(text) = std::str::from_utf8(&output.stdout) {
                if let Some(plib) = parse_pllibdir(text) {
                    let plib_path = PathBuf::from(plib);
                    // The exposed path is usually `<root>/library`; walk up
                    // one level so we can also reach `boot/`.
                    let parent = plib_path.parent().map(PathBuf::from).unwrap_or(plib_path);
                    if looks_like_swipl_source(&parent) {
                        return Some(parent);
                    }
                }
            }
        }
    }

    None
}

/// A SWI-Prolog source tree always exposes `library/` with `.pl` files.
/// The `boot/` directory is present in checkouts but not always in trimmed
/// installs (some Linux packagers compile boot files into a `.qlf` archive
/// and drop the `.pl` originals); detection only requires `library/`.
fn looks_like_swipl_source(p: &Path) -> bool {
    if !p.is_dir() {
        return false;
    }
    let library = p.join("library");
    if !library.is_dir() {
        return false;
    }
    // Sanity probe: a populated SWI library always has `lists.pl`.
    library.join("lists.pl").is_file()
}

fn parse_pllibdir(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        // Common shapes: `PLLIBDIR="/usr/lib/swi-prolog/library";` (sh-eval
        // form) and bare `PLLIBDIR=/usr/lib/swi-prolog/library`.
        if let Some(rest) = line.strip_prefix("PLLIBDIR=") {
            let val = rest
                .trim_end_matches(';')
                .trim_matches('"')
                .trim_matches('\'');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

fn home_dir() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("HOME") {
        let p = PathBuf::from(h);
        if p.is_dir() { return Some(p) }
    }
    if let Some(h) = std::env::var_os("USERPROFILE") {
        let p = PathBuf::from(h);
        if p.is_dir() { return Some(p) }
    }
    None
}

// ---------------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------------

fn walk_prolog_tree(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &dep.module_path, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, module_prefix: &str, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= 16 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            // SWI-Prolog has a few subdirectories (`clp/`, `http/`, `dialect/`,
            // `unicode/`, ...). Walk them; skip `.git` and other dotdirs.
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') { continue }
                // Skip the `pldoc/` test fixtures — they're contrived
                // documentation samples, not real predicates.
                if matches!(name, "pldoc_test" | "test") { continue }
            }
            walk_dir(&path, module_prefix, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            // SWI uses `.pl` for source; some boot files are `.pl` too. We
            // skip `.qlf` (compiled), `.so`/`.dll` (foreign), and `.html`/
            // `.md` (documentation).
            if !name.ends_with(".pl") {
                continue;
            }
            let display = path.to_string_lossy().replace('\\', "/");
            let rel = format!("ext:{}:{}", ECOSYSTEM_TAG, display);
            out.push(WalkedFile {
                relative_path: rel,
                absolute_path: path.clone(),
                language: "prolog",
            });
        }
    }
}

#[cfg(test)]
#[path = "prolog_runtime_tests.rs"]
mod tests;
