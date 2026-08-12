// =============================================================================
// ecosystem/go_stdlib.rs — Go stdlib (stdlib ecosystem)
//
// Probes `go env GOROOT`, locates `$GOROOT/src/`, and registers one
// `ExternalDepRoot` per directory that declares a Go package (fmt, strings,
// net/http, ...). Go import paths are siblings, not nested — `net` and
// `net/http` are independent packages that happen to share a filesystem
// prefix — so each package directory gets its own root keyed by its exact
// import path, matching how refs carry `module: Some("net/http")` and how
// `SymbolLocationIndex::locate(module, name)` does an exact-string match.
// Their symbols come through the regular Go extractor (package declarations
// → qname prefix), so `fmt.Printf` in user code lines up with the qname
// extracted from `$GOROOT/src/fmt/print.go`.
//
// Activation is `LanguagePresent("go")` — no manifest required.
// Degrades to empty discovery if the Go toolchain isn't on PATH or
// `go env GOROOT` returns nothing.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tracing::debug;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("go-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "go-stdlib";
const LANGUAGES: &[&str] = &["go"];

pub struct GoStdlibEcosystem;

impl Ecosystem for GoStdlibEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Stdlib
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("go")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_go_stdlib_roots()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_go_tree(dep)
    }

    fn supports_reachability(&self) -> bool {
        true
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }

    fn build_symbol_index(
        &self,
        dep_roots: &[crate::ecosystem::externals::ExternalDepRoot],
    ) -> crate::ecosystem::symbol_index::SymbolLocationIndex {
        // Reuse the go_mod builder — stdlib Go files follow the same
        // package-declaration layout, and each root here is already scoped
        // to exactly one package directory (see `discover_go_stdlib_roots`),
        // so the module_path it keys every file under is correct.
        super::go_mod::build_go_symbol_index(dep_roots)
    }
}

impl ExternalSourceLocator for GoStdlibEcosystem {
    fn ecosystem(&self) -> &'static str {
        LEGACY_ECOSYSTEM_TAG
    }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_go_stdlib_roots()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_go_tree(dep)
    }
}

fn discover_go_stdlib_roots() -> Vec<ExternalDepRoot> {
    let Some(goroot) = goroot() else {
        debug!("go-stdlib: GOROOT not found");
        return Vec::new();
    };
    let src_dir = goroot.join("src");
    if !src_dir.is_dir() {
        debug!("go-stdlib: {} missing", src_dir.display());
        return Vec::new();
    }
    let mut roots = Vec::new();
    collect_package_roots(&src_dir, &src_dir, &mut roots, 0);
    debug!(
        "go-stdlib registered {} package roots under {}",
        roots.len(),
        src_dir.display()
    );
    roots
}

/// Recursively find every directory under `src_root` that defines a Go
/// package — contains at least one non-test `.go` file — and register one
/// `ExternalDepRoot` per directory, keyed by its import path relative to
/// `src_root` (`net/http`, not `net`). `requested_imports` is set to the
/// package's own import path: this narrows the shared
/// `resolve_go_requested_packages` (go_mod) to exactly this directory
/// instead of its unbounded recursive fallback, which has no concept of
/// sibling package boundaries and would fold a nested package's symbols
/// into its parent's module key.
fn collect_package_roots(
    dir: &Path,
    src_root: &Path,
    roots: &mut Vec<ExternalDepRoot>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs = Vec::new();
    let mut has_go_file = false;
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Skip the Go compiler and its internal tooling — millions of
                // symbols that user code never imports.
                if matches!(name, "cmd" | "testdata" | "internal" | "vendor") {
                    continue;
                }
                if name.starts_with('.') || name.starts_with('_') {
                    continue;
                }
            }
            subdirs.push(path);
        } else if ft.is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".go") && !name.ends_with("_test.go") {
                    has_go_file = true;
                }
            }
        }
    }
    if has_go_file {
        if let Ok(rel) = dir.strip_prefix(src_root) {
            let module_path = rel.to_string_lossy().replace('\\', "/");
            if !module_path.is_empty() {
                roots.push(ExternalDepRoot {
                    module_path: module_path.clone(),
                    version: String::new(),
                    root: dir.to_path_buf(),
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: vec![module_path],
                });
            }
        }
    }
    for sub in subdirs {
        collect_package_roots(&sub, src_root, roots, depth + 1);
    }
}

fn goroot() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_GOROOT") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Some(env_goroot) = std::env::var_os("GOROOT") {
        let p = PathBuf::from(env_goroot);
        if p.is_dir() {
            return Some(p);
        }
    }
    let output = Command::new("go").args(["env", "GOROOT"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let p = PathBuf::from(trimmed);
    if p.is_dir() {
        Some(p)
    } else {
        None
    }
}

/// Enumerate the `.go` files directly in one stdlib package's directory.
/// Non-recursive: `discover_go_stdlib_roots` already registers one root per
/// package directory, so descending into subdirectories here would
/// attribute a sibling package's files to this package's module_path.
fn walk_go_tree(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dep.root) else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".go") || name.ends_with("_test.go") {
            continue;
        }
        if !super::go_platform::file_matches_host(name) {
            continue;
        }
        let virtual_path = format!("ext:go-stdlib/{}/{}", dep.module_path, name);
        out.push(WalkedFile {
            relative_path: virtual_path,
            absolute_path: path,
            language: "go",
        });
    }
    out
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<GoStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(GoStdlibEcosystem)).clone()
}

#[cfg(test)]
#[path = "go_stdlib_tests.rs"]
mod tests;
