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
// The plain-text `.R` source for R's base packages is available only
// in R's *source distribution* tarball at
// `<R-src>/src/library/<pkg>/R/*.R`. Most users don't have an R source
// checkout on disk, so this walker only fires when the user explicitly
// points at one via `BEARWISDOM_R_SRC`. When the env var is unset or
// the path doesn't contain `src/library/`, the walker emits nothing
// and `mean`, `nrow`, `paste`, `lapply`, etc. stay unresolved.
//
// Activation: `LanguagePresent("r")` — every R project uses these
// names, but the walker silently degrades when the source tarball is
// not on disk (consistent with the trait doc's "missing toolchain"
// degrade-honestly behaviour).
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("r-stdlib");
const TAG: &str = "r-stdlib";
const LANGUAGES: &[&str] = &["r"];

/// R base packages shipped in `<R-src>/src/library/`.
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
        walk_r_tree(dep)
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
        walk_r_tree(dep)
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

fn discover_r_stdlib() -> Vec<ExternalDepRoot> {
    let Some(r_src) = probe_r_source() else {
        debug!("r-stdlib: no R source distribution probed (set BEARWISDOM_R_SRC)");
        return Vec::new();
    };
    let library_root = r_src.join("src").join("library");
    if !library_root.is_dir() {
        debug!(
            "r-stdlib: BEARWISDOM_R_SRC={} does not contain src/library/",
            r_src.display()
        );
        return Vec::new();
    }
    debug!("r-stdlib: using {}", library_root.display());
    vec![ExternalDepRoot {
        module_path: "r-stdlib".to_string(),
        version: String::new(),
        root: library_root,
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn probe_r_source() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_R_SRC") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p); }
    }
    None
}

// ---------------------------------------------------------------------------
// Walk
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

#[cfg(test)]
#[path = "r_stdlib_tests.rs"]
mod tests;
