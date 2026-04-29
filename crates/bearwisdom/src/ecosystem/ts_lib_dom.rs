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
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
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

    fn supports_reachability(&self) -> bool { true }

    // lib.*.d.ts is the fallback for "bare method call whose receiver type we
    // couldn't infer": `x.trim()`, `event.preventDefault()`, `setTimeout(...)`.
    // A demand-filtered index skips most of lib.dom.d.ts (it's 1.8MB of
    // declarations), so those calls land in unresolved even though the
    // answer is right there in the ingested file. Full indexing costs a
    // one-time parse but lets the resolver's name lookup find every
    // top-level DOM/ES symbol without guessing.
    fn uses_demand_driven_parse(&self) -> bool { false }

    fn build_symbol_index(
        &self,
        dep_roots: &[crate::ecosystem::externals::ExternalDepRoot],
    ) -> crate::ecosystem::symbol_index::SymbolLocationIndex {
        super::npm::build_npm_symbol_index(dep_roots)
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

/// Synthetic module name used for TypeScript core lib files
/// (`lib.dom.d.ts`, `lib.es*.d.ts`, etc.). They declare runtime globals
/// rather than members of a real npm package, so we route them through a
/// reserved prefix that the resolver and `ts_post_process_external` both
/// recognise — keeping their symbols un-prefixed instead of mangling them
/// under whatever an absolute Windows path happens to look like.
pub const TS_LIB_SYNTHETIC_MODULE: &str = "__ts_lib__";

fn discover_ts_lib_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
    let mut roots = Vec::new();

    // Explicit override for CI / unusual layouts.
    if let Some(explicit) = std::env::var_os("BEARWISDOM_TS_LIB_DIR") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            roots.push(make_root(&p, TS_LIB_SYNTHETIC_MODULE));
        }
    }

    if roots.is_empty() {
        if let Some(dir) = find_typescript_lib(project_root) {
            roots.push(make_root(&dir, TS_LIB_SYNTHETIC_MODULE));
        }
    }

    // @types/node is a separate dep root so its files tag as Node rather
    // than DOM.
    if let Some(node_types) = find_types_node(project_root) {
        roots.push(make_root(&node_types, "@types/node"));
    }

    if roots.is_empty() {
        debug!("ts-lib-dom: no TypeScript lib.*.d.ts found for {}", project_root.display());
    }
    roots
}

fn make_root(dir: &Path, module_path: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: module_path.to_string(),
        version: String::new(),
        root: dir.to_path_buf(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

fn find_typescript_lib(project_root: &Path) -> Option<PathBuf> {
    // Project-local: walk up from project_root checking every ancestor's
    // node_modules/typescript/lib. Covers monorepos with hoisted installs.
    for dir in ancestors_with_node_modules(project_root) {
        let lib = dir.join("typescript").join("lib");
        if lib.is_dir() { return Some(lib); }
    }
    // User-global npm install — common fallback for pure-backend projects
    // (ASP.NET, Django, Rails) that embed a handful of JS files under
    // `wwwroot/`/`public/`/`static/` without a project-level npm install.
    // Without this, the natural resolution pipeline can't see
    // `setTimeout`/`Array.prototype.map`/etc. and those references fall
    // through to unresolved. Probes Windows (`APPDATA\npm`), Unix
    // (`/usr/lib`, `/usr/local/lib`, `~/.npm-global`, `~/.nvm/current`).
    for root in global_npm_roots() {
        let lib = root.join("typescript").join("lib");
        if lib.is_dir() { return Some(lib); }
    }
    None
}

fn global_npm_roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        out.push(PathBuf::from(appdata).join("npm").join("node_modules"));
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        out.push(home.join(".npm-global").join("lib").join("node_modules"));
        out.push(home.join(".nvm").join("current").join("lib").join("node_modules"));
        out.push(home.join("node_modules"));
    }
    out.push(PathBuf::from("/usr/local/lib/node_modules"));
    out.push(PathBuf::from("/usr/lib/node_modules"));
    out.push(PathBuf::from("/opt/homebrew/lib/node_modules"));
    out
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
    walk_dir(dep, &dep.root, &mut out, 0);
    out
}

fn walk_dir(dep: &ExternalDepRoot, dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
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
            walk_dir(dep, &path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".d.ts") { continue }
            let Ok(rel) = path.strip_prefix(&dep.root) else { continue };
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:ts:{}/{}", dep.module_path, rel_s),
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
