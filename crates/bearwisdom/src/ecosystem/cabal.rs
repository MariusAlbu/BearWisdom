// =============================================================================
// ecosystem/cabal.rs — Cabal ecosystem (Haskell)
//
// Phase 2 + 3: consolidates `indexer/externals/haskell.rs`. There's no
// separate `manifest/cabal.rs` — the .cabal file parsing lives here.
// Probes both Stack (`.stack-work/install/`) and Cabal
// (`~/.cabal/store/ghc-<ver>/`) package stores.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::debug;
use tree_sitter::{Node, Parser};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("cabal");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["haskell"];
const LEGACY_ECOSYSTEM_TAG: &str = "haskell";

pub struct CabalEcosystem;

impl Ecosystem for CabalEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("haskell"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_haskell_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_haskell_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }
    fn resolve_import(
        &self, dep: &ExternalDepRoot, _p: &str, _s: &[&str],
    ) -> Vec<WalkedFile> { walk_haskell_narrowed(dep) }
    fn resolve_symbol(
        &self, dep: &ExternalDepRoot, _f: &str,
    ) -> Vec<WalkedFile> { walk_haskell_narrowed(dep) }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_haskell_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for CabalEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_haskell_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_haskell_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<CabalEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(CabalEcosystem)).clone()
}

// ===========================================================================
// Discovery
// ===========================================================================

pub fn discover_haskell_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let declared = parse_cabal_build_depends(project_root);
    if declared.is_empty() { return Vec::new() }

    let user_imports: Vec<String> = collect_haskell_user_imports(project_root)
        .into_iter()
        .collect();

    let stack_root = project_root.join(".stack-work");
    if stack_root.is_dir() {
        let roots = find_haskell_stack_deps(&stack_root, &declared, &user_imports);
        if !roots.is_empty() {
            debug!("Haskell: {} roots via Stack", roots.len());
            return roots;
        }
    }

    let roots = find_haskell_cabal_deps(&declared, &user_imports);
    debug!("Haskell: {} roots via Cabal", roots.len());
    roots
}

pub fn parse_cabal_build_depends(project_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(project_root) else { return Vec::new() };
    let cabal_file = entries
        .flatten()
        .find(|e| e.path().extension().and_then(|x| x.to_str()) == Some("cabal"));
    let Some(cabal_entry) = cabal_file else { return Vec::new() };
    let Ok(content) = std::fs::read_to_string(cabal_entry.path()) else { return Vec::new() };

    let mut deps = Vec::new();
    let mut in_build_depends = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.to_lowercase().starts_with("build-depends:") {
            in_build_depends = true;
            let rest = trimmed["build-depends:".len()..].trim();
            if !rest.is_empty() { deps.extend(parse_cabal_dep_list(rest)) }
            continue;
        }
        if in_build_depends {
            if !line.starts_with(' ') && !line.starts_with('\t') && !trimmed.starts_with(',') {
                in_build_depends = false;
                continue;
            }
            deps.extend(parse_cabal_dep_list(trimmed));
        }
    }
    deps.sort();
    deps.dedup();
    deps
}

fn parse_cabal_dep_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|chunk| {
            chunk.trim().split_whitespace().next().unwrap_or("").trim().to_string()
        })
        .filter(|name| !name.is_empty() && name != "base")
        .collect()
}

fn find_haskell_stack_deps(
    stack_work: &Path,
    declared: &[String],
    user_imports: &[String],
) -> Vec<ExternalDepRoot> {
    let install = stack_work.join("install");
    if !install.is_dir() { return Vec::new() }
    let mut roots = Vec::new();
    let Ok(platforms) = std::fs::read_dir(&install) else { return Vec::new() };
    for platform in platforms.flatten() {
        let Ok(hashes) = std::fs::read_dir(platform.path()) else { continue };
        for hash in hashes.flatten() {
            let lib = hash.path().join("lib");
            if !lib.is_dir() { continue }
            let Ok(ghc_vers) = std::fs::read_dir(&lib) else { continue };
            for ghc_ver in ghc_vers.flatten() {
                find_haskell_pkgs_in_dir(&ghc_ver.path(), declared, user_imports, &mut roots);
            }
        }
    }
    roots
}

fn find_haskell_cabal_deps(
    declared: &[String],
    user_imports: &[String],
) -> Vec<ExternalDepRoot> {
    let mut candidates = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let store1 = home.join(".cabal").join("store");
        let store2 = home.join(".local").join("state").join("cabal").join("store");
        for store in [store1, store2] {
            if store.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&store) {
                    for e in entries.flatten() {
                        if e.path().is_dir() { candidates.push(e.path()) }
                    }
                }
            }
        }
    }
    let mut roots = Vec::new();
    for ghc_dir in &candidates {
        find_haskell_pkgs_in_dir(ghc_dir, declared, user_imports, &mut roots);
    }
    roots
}

fn find_haskell_pkgs_in_dir(
    dir: &Path,
    declared: &[String],
    user_imports: &[String],
    roots: &mut Vec<ExternalDepRoot>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        for dep in declared {
            let prefix = format!("{dep}-");
            if name_str.starts_with(&prefix) && entry.path().is_dir() {
                let version = name_str[prefix.len()..].to_string();
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version,
                    root: entry.path(),
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: user_imports.to_vec(),
                });
                break;
            }
        }
    }
}

// R3 — `import Data.List` / `import qualified X.Y as Z` scanner. Maps each
// dotted module to a `Data/List.hs` tail.

fn collect_haskell_user_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    scan_haskell_imports(project_root, &mut out, 0);
    out
}

fn scan_haskell_imports(
    dir: &Path,
    out: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 12 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, ".git" | ".stack-work" | "dist-newstyle" | "test" | "tests" | "bench")
                    || name.starts_with('.') { continue }
            }
            scan_haskell_imports(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(name.ends_with(".hs") || name.ends_with(".lhs")) { continue }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            extract_haskell_imports(&content, out);
        }
    }
}

fn extract_haskell_imports(content: &str, out: &mut std::collections::HashSet<String>) {
    for raw in content.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("import ") else { continue };
        let rest = rest.trim_start_matches("qualified ").trim();
        let head = rest
            .split(|c: char| c == ' ' || c == '\t' || c == '(' || c == ';')
            .next()
            .unwrap_or("")
            .trim();
        if head.is_empty() { continue }
        if !head.chars().next().map_or(false, |c| c.is_ascii_uppercase()) { continue }
        out.insert(head.to_string());
    }
}

fn haskell_module_to_path_tail(module: &str) -> Option<String> {
    let cleaned = module.trim();
    if cleaned.is_empty() { return None }
    Some(format!("{}.hs", cleaned.replace('.', "/")))
}

fn walk_haskell_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() { return walk_haskell_root(dep); }
    let tails: std::collections::HashSet<String> = dep
        .requested_imports
        .iter()
        .filter_map(|m| haskell_module_to_path_tail(m))
        .collect();
    if tails.is_empty() { return walk_haskell_root(dep); }

    let mut out = Vec::new();
    walk_haskell_narrowed_dir(&dep.root, &dep.root, dep, &tails, &mut out, 0);
    out
}

fn walk_haskell_narrowed_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    tails: &std::collections::HashSet<String>,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut subdirs: Vec<PathBuf> = Vec::new();
    let mut dir_files: Vec<(PathBuf, String)> = Vec::new();
    let mut any_match = false;

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "bench" | "dist-newstyle" | ".stack-work")
                    || name.starts_with('.') { continue }
            }
            subdirs.push(path);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".hs") { continue }
            if name.ends_with("Spec.hs") || name.ends_with("Test.hs") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if tails.iter().any(|t| rel_sub.ends_with(t)) { any_match = true; }
            dir_files.push((path, rel_sub));
        }
    }

    if any_match {
        for (path, rel_sub) in dir_files {
            out.push(WalkedFile {
                relative_path: format!("ext:haskell:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "haskell",
            });
        }
    }
    for sub in subdirs {
        walk_haskell_narrowed_dir(&sub, root, dep, tails, out, depth + 1);
    }
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_haskell_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "bench" | "dist-newstyle" | ".stack-work")
                    || name.starts_with('.')
                { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".hs") { continue }
            if name.ends_with("Spec.hs") || name.ends_with("Test.hs") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:haskell:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "haskell",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------

fn build_haskell_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_haskell_root(dep) {
            work.push((dep.module_path.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }
    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_haskell_header(&src)
                .into_iter()
                .map(|name| (module.clone(), name, wf.absolute_path.clone()))
                .collect()
        })
        .collect();
    let mut index = SymbolLocationIndex::new();
    for batch in per_file {
        for (module, name, file) in batch {
            index.insert(module, name, file);
        }
    }
    index
}

/// Header-only tree-sitter scan of a Haskell source file. Records top-level
/// `data`, `newtype`, `type`, `class`, and function signatures. Function
/// bodies are not descended.
fn scan_haskell_header(source: &str) -> Vec<String> {
    let language = tree_sitter_haskell::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_haskell_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_haskell_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "data_type"
        | "data"
        | "newtype"
        | "type_synomym"
        | "type_family"
        | "class"
        | "function"
        | "signature"
        | "bind"
        | "decl" => {
            if let Some(name_node) = node
                .child_by_field_name("name")
                .or_else(|| node.child_by_field_name("variable"))
            {
                if let Ok(t) = name_node.utf8_text(bytes) {
                    out.push(t.to_string());
                }
            }
        }
        // Haskell grammar wraps multi-form decls in a `declarations` / `decls`
        // block. Recurse once.
        "declarations" | "haskell" | "module" => {
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                collect_haskell_top_level_name(&inner, bytes, out);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let c = CabalEcosystem;
        assert_eq!(c.id(), ID);
        assert_eq!(Ecosystem::kind(&c), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&c), &["haskell"]);
    }

    #[test]
    fn legacy_locator_tag_is_haskell() {
        assert_eq!(ExternalSourceLocator::ecosystem(&CabalEcosystem), "haskell");
    }

    #[test]
    fn haskell_parses_cabal_build_depends() {
        let tmp = std::env::temp_dir().join("bw-test-cabal-deps");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("test.cabal"), r#"
cabal-version: 2.0
name: test
version: 1.0
library
  build-depends:
    aeson >= 2.0,
    text,
    bytestring
"#).unwrap();
        let deps = parse_cabal_build_depends(&tmp);
        assert_eq!(deps, vec!["aeson", "bytestring", "text"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }

    #[test]
    fn haskell_extracts_imports_with_qualified() {
        let mut out = std::collections::HashSet::new();
        extract_haskell_imports(
            "module Main where\nimport Data.List\nimport qualified Data.Map.Strict as M\nimport Control.Monad (when)\n",
            &mut out,
        );
        assert!(out.contains("Data.List"));
        assert!(out.contains("Data.Map.Strict"));
        assert!(out.contains("Control.Monad"));
    }

    #[test]
    fn haskell_module_path_conversion() {
        assert_eq!(haskell_module_to_path_tail("Data.List"), Some("Data/List.hs".to_string()));
        assert_eq!(haskell_module_to_path_tail("Control.Monad.State"), Some("Control/Monad/State.hs".to_string()));
    }
}
