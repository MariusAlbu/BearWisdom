// =============================================================================
// ecosystem/pypi.rs — PyPI ecosystem (Python)
//
// Phase 2 + 3 combined: consolidates the external-source locator
// (`indexer/externals/python.rs`) and the manifest reader
// (`indexer/manifest/pyproject.rs`) into a single ecosystem module.
// =============================================================================

use std::path::Path;
use std::sync::Arc;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("pypi");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["python"];
pub(super) const LEGACY_ECOSYSTEM_TAG: &str = "python";

pub struct PypiEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait impl
// ---------------------------------------------------------------------------

impl Ecosystem for PypiEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        // pyproject.toml is the modern marker; setup.py persists in older
        // projects. Both legitimately mark a package root.
        &[
            ("pyproject.toml", "python"),
            ("setup.py",       "python"),
        ]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["__pycache__", ".venv", "venv", ".tox", ".pytest_cache",
          ".mypy_cache", ".ruff_cache", "site-packages", ".eggs"]
    }

    fn activation(&self) -> EcosystemActivation {
        // Project deps via `pyproject.toml` (or `requirements.txt`,
        // `Pipfile`). The Python toolchain (stdlib) belongs to
        // `cpython-stdlib`; pypi only resolves declared third-party deps.
        // Dropping the LanguagePresent shotgun is correct per the trait
        // doc.
        EcosystemActivation::ManifestMatch
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_python_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_python_external_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        // Python packages live as either a directory (with __init__.py) or a
        // single-file module (pkg.py). For directory packages, start from
        // __init__.py and follow relative-import expansion bounded at a
        // small depth. Single-file modules return just themselves.
        let mut out = resolve_python_package_entry(dep);

        // For dotted requests (`from django.test import TestCase` where
        // dep.module_path is `django`), also walk the requested
        // sub-package's `__init__.py`. The package's top-level
        // `__init__.py` rarely re-exports its sub-modules wholesale
        // (Django's `django/__init__.py` doesn't `from .test import *`),
        // so demand-driven users requesting `django.test.TestCase` get
        // nothing without this targeted pull.
        if dep.root.is_dir() && package.contains('.') {
            // Only consider subpath segments that match `dep.module_path`'s
            // leading segment — `django.test.TestCase` is rooted at
            // `django` if `dep.module_path == "django"`. Anything else is
            // a different package's request.
            let mut iter = package.splitn(2, '.');
            let first = iter.next().unwrap_or("");
            let rest = iter.next().unwrap_or("");
            if first == dep.module_path && !rest.is_empty() {
                let mut subdir = dep.root.clone();
                let mut parts = rest.split('.').peekable();
                while let Some(part) = parts.next() {
                    subdir = subdir.join(part);
                    let init = subdir.join("__init__.py");
                    if init.is_file() {
                        let mut seen: std::collections::HashSet<std::path::PathBuf> =
                            out.iter().map(|wf| wf.absolute_path.clone()).collect();
                        expand_python_reexports_into(
                            dep, &dep.root, &init, &mut out, &mut seen, 0,
                        );
                    }
                    // Final segment may be a leaf .py file.
                    if parts.peek().is_none() {
                        let leaf = subdir.with_extension("py");
                        if leaf.is_file() {
                            let mut seen: std::collections::HashSet<std::path::PathBuf> =
                                out.iter().map(|wf| wf.absolute_path.clone()).collect();
                            expand_python_reexports_into(
                                dep, &dep.root, &leaf, &mut out, &mut seen, 0,
                            );
                        }
                    }
                }
            }
        }

        out
    }

    fn resolve_symbol(
        &self,
        dep: &ExternalDepRoot,
        _fqn: &str,
    ) -> Vec<WalkedFile> {
        // Same entry point. Re-exports within the package are expanded by
        // resolve_python_package_entry; deeper fqn-specific walking is a
        // later optimization.
        resolve_python_package_entry(dep)
    }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_python_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for PypiEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_python_externals(project_root)
    }

    fn locate_roots_for_package(
        &self,
        workspace_root: &Path,
        package_abs_path: &Path,
        package_id: i64,
    ) -> Vec<ExternalDepRoot> {
        let mut roots = discover_python_externals_scoped(workspace_root, package_abs_path);
        for r in &mut roots {
            r.package_id = Some(package_id);
        }
        roots
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_python_external_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PypiEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(PypiEcosystem)).clone()
}

mod discovery;
mod manifest;
mod reachability;
mod symbol_index;
mod walk;

pub use discovery::{
    discover_python_externals, discover_python_externals_scoped, find_python_site_packages,
    normalize_python_dep_name,
};
pub use manifest::{
    parse_pipfile_deps, parse_pyproject_deps, parse_requirements_txt, PyProjectManifest,
};
pub use walk::walk_python_external_root;
pub(crate) use symbol_index::build_python_symbol_index;

use reachability::{expand_python_reexports_into, resolve_python_package_entry};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
