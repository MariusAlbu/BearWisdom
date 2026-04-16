// =============================================================================
// ecosystem/ts_lib_dom.rs — TypeScript lib.*.d.ts + Node types (stdlib)
//
// Indexes `lib.dom.d.ts`, `lib.es*.d.ts`, and adjacent `lib.*.d.ts`
// declaration files shipped with the TypeScript compiler. These describe
// every DOM type (Document, HTMLElement, Request, fetch, ...) and ES
// global API (Map, Promise, Array.prototype.*), which are the top
// source of unresolved refs in browser-oriented TypeScript/JavaScript
// projects.
//
// Probe strategy:
//   1. $BEARWISDOM_TS_LIB_DIR → explicit dir of lib.*.d.ts files
//   2. <project>/node_modules/typescript/lib/
//   3. Walk up ancestor node_modules looking for typescript/lib/ (hoisted
//      workspace layouts)
//   4. @types/node under node_modules/@types/node/ for Node.js globals.
//
// Activation: LanguagePresent ts / tsx / js / vue / svelte / angular / astro.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::indexer::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("ts-lib-dom");
const LEGACY_ECOSYSTEM_TAG: &str = "ts-lib-dom";
const LANGUAGES: &[&str] = &["typescript", "tsx", "javascript", "vue", "svelte", "angular", "astro"];

pub struct TsLibDomEcosystem;

impl Ecosystem for TsLibDomEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("typescript"),
            EcosystemActivation::LanguagePresent("tsx"),
            EcosystemActivation::LanguagePresent("javascript"),
            EcosystemActivation::LanguagePresent("vue"),
            EcosystemActivation::LanguagePresent("svelte"),
            EcosystemActivation::LanguagePresent("angular"),
            EcosystemActivation::LanguagePresent("astro"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_ts_lib_roots(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ts_lib(dep)
    }
}

impl ExternalSourceLocator for TsLibDomEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_ts_lib_roots(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ts_lib(dep)
    }
}

fn discover_ts_lib_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
    let mut roots = Vec::new();

    // Explicit override for CI / unusual layouts.
    if let Some(explicit) = std::env::var_os("BEARWISDOM_TS_LIB_DIR") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            roots.push(make_root(&p, "ts-lib"));
        }
    }

    if roots.is_empty() {
        if let Some(dir) = find_typescript_lib(project_root) {
            roots.push(make_root(&dir, "ts-lib"));
        }
    }

    // @types/node is a separate dep root so its files tag as Node rather
    // than DOM.
    if let Some(node_types) = find_types_node(project_root) {
        roots.push(make_root(&node_types, "types-node"));
    }

    if roots.is_empty() {
        debug!("ts-lib-dom: no TypeScript lib.*.d.ts found for {}", project_root.display());
    }
    roots
}

fn make_root(dir: &Path, tag: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: tag.to_string(),
        version: String::new(),
        root: dir.to_path_buf(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
    }
}

fn find_typescript_lib(project_root: &Path) -> Option<PathBuf> {
    for dir in ancestors_with_node_modules(project_root) {
        let lib = dir.join("typescript").join("lib");
        if lib.is_dir() { return Some(lib); }
    }
    None
}

fn find_types_node(project_root: &Path) -> Option<PathBuf> {
    for dir in ancestors_with_node_modules(project_root) {
        let p = dir.join("@types").join("node");
        if p.is_dir() { return Some(p); }
    }
    None
}

fn ancestors_with_node_modules(project_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cur: Option<&Path> = Some(project_root);
    while let Some(dir) = cur {
        let nm = dir.join("node_modules");
        if nm.is_dir() { out.push(nm); }
        cur = dir.parent();
    }
    out
}

fn walk_ts_lib(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= 8 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "__tests__") { continue }
                if name.starts_with('.') { continue }
            }
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".d.ts") { continue }
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:ts:{}", display),
                absolute_path: path,
                language: "typescript",
            });
        }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<TsLibDomEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(TsLibDomEcosystem)).clone()
}
