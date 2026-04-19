// =============================================================================
// ecosystem/go_stdlib.rs — Go stdlib (stdlib ecosystem)
//
// Probes `go env GOROOT`, locates `$GOROOT/src/`, and walks it as one
// external dep root. Each top-level directory there is a Go stdlib
// package (fmt, strings, io, net/http, ...); their symbols come through
// the regular Go extractor (package declarations → qname prefix), so
// `fmt.Printf` in user code lines up with the qname extracted from
// `$GOROOT/src/fmt/print.go`.
//
// Activation is `LanguagePresent("go")` — no manifest required.
// Degrades to empty discovery if the Go toolchain isn't on PATH or
// `go env GOROOT` returns nothing.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("go-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "go-stdlib";
const LANGUAGES: &[&str] = &["go"];

pub struct GoStdlibEcosystem;

impl Ecosystem for GoStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("go")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_go_stdlib_roots()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_go_tree(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(
        &self,
        dep_roots: &[crate::ecosystem::externals::ExternalDepRoot],
    ) -> crate::ecosystem::symbol_index::SymbolLocationIndex {
        // Reuse the go_mod builder — stdlib Go files follow the same
        // package-declaration layout.
        super::go_mod::build_go_symbol_index(dep_roots)
    }
}

impl ExternalSourceLocator for GoStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
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
    debug!("go-stdlib registered at {}", src_dir.display());
    vec![ExternalDepRoot {
        module_path: "go-stdlib".to_string(),
        version: String::new(),
        root: src_dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn goroot() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_GOROOT") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p) }
    }
    if let Some(env_goroot) = std::env::var_os("GOROOT") {
        let p = PathBuf::from(env_goroot);
        if p.is_dir() { return Some(p) }
    }
    let output = Command::new("go")
        .args(["env", "GOROOT"])
        .output()
        .ok()?;
    if !output.status.success() { return None }
    let path = String::from_utf8(output.stdout).ok()?;
    let trimmed = path.trim();
    if trimmed.is_empty() { return None }
    let p = PathBuf::from(trimmed);
    if p.is_dir() { Some(p) } else { None }
}

fn walk_go_tree(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Skip the Go compiler and its internal tooling — millions of
                // symbols that user code never imports.
                if matches!(name, "cmd" | "testdata" | "internal" | "vendor") { continue }
                if name.starts_with('.') || name.starts_with('_') { continue }
            }
            walk_dir(&path, root, dep, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".go") { continue }
            if name.ends_with("_test.go") { continue }
            if !super::go_platform::file_matches_host(name) { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:go-stdlib/{}", rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "go",
            });
        }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<GoStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(GoStdlibEcosystem)).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let e = GoStdlibEcosystem;
        assert_eq!(e.id(), ID);
        assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
        assert_eq!(Ecosystem::languages(&e), &["go"]);
    }

    #[test]
    fn legacy_locator_tag() {
        assert_eq!(ExternalSourceLocator::ecosystem(&GoStdlibEcosystem), "go-stdlib");
    }
}
