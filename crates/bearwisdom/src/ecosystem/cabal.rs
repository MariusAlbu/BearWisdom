// =============================================================================
// ecosystem/cabal.rs — Cabal ecosystem (Haskell)
//
// Phase 2 + 3: consolidates `indexer/externals/haskell.rs`. There's no
// separate `manifest/cabal.rs` — the .cabal file parsing lives here.
// Probes Stack (`.stack-work/install/`) and Cabal package stores.
// Cabal store paths searched: `$HOME/.cabal/store`, `$HOME/.local/state/cabal/store`
// (Linux/macOS XDG), `%LOCALAPPDATA%\cabal\store` (Windows default for cabal 3.x).
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

    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        &[("cabal.project", "haskell")]
    }

    fn workspace_package_extensions(&self) -> &'static [(&'static str, &'static str)] {
        &[(".cabal", "haskell")]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["dist", "dist-newstyle", ".stack-work"]
    }

    fn activation(&self) -> EcosystemActivation {
        // Project deps via `*.cabal` `build-depends`. A bare directory of
        // `.hs` files with no manifest can't be resolved against external
        // Hackage coordinates — no constraints, no package set. Dropping
        // the LanguagePresent shotgun is correct per the trait doc.
        EcosystemActivation::ManifestMatch
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

    /// Pre-pull the top-level module files that the user's project imports
    /// directly. When those files are parsed by the demand pipeline, their
    /// `import` declarations (including re-exports from sibling packages like
    /// `hspec → hspec-core`) emit `Imports` refs. The demand BFS resolves
    /// those refs against the symbol index (now keyed by Haskell module name
    /// as well as package name) and pulls the transitive definitions — giving
    /// bare names like `it` and `describe` a path to their defining file.
    fn demand_pre_pull(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> Vec<crate::walker::WalkedFile> {
        dep_roots
            .iter()
            .flat_map(|dep| walk_haskell_narrowed(dep))
            .collect()
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
// Manifest reader
// ===========================================================================

/// Surfaces `*.cabal` `build-depends` entries in
/// `ProjectContext.manifests[ManifestKind::Cabal]`. The Stack flow
/// (`stack.yaml`) defers to the same `.cabal` build-depends list — Stack
/// just adds an alternative storage location, not a different dep source.
pub struct CabalManifest;

impl crate::ecosystem::manifest::ManifestReader for CabalManifest {
    fn kind(&self) -> crate::ecosystem::manifest::ManifestKind {
        crate::ecosystem::manifest::ManifestKind::Cabal
    }

    fn read(&self, project_root: &Path) -> Option<crate::ecosystem::manifest::ManifestData> {
        let deps = parse_cabal_build_depends(project_root);
        if deps.is_empty() { return None }
        let mut data = crate::ecosystem::manifest::ManifestData::default();
        data.dependencies = deps.into_iter().collect();
        Some(data)
    }
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

    // Boot libraries that every Haskell project transitively depends on
    // (`base` re-exports identifiers actually defined in `ghc-internal` /
    // `ghc-prim` / `ghc-bignum`) rarely appear in user `build-depends`.
    // Read GHC's own package database to discover them — the `.conf` files
    // under `<ghc-libdir>/package.conf.d/` enumerate every package the
    // toolchain has registered. No hand-maintained list, no probe-and-pray:
    // we ask the compiler what it ships with.
    let toolchain_pkgs = discover_ghc_registered_packages();
    let mut implicit: Vec<String> = declared.clone();
    for pkg in &toolchain_pkgs {
        if !implicit.iter().any(|d| d == pkg) {
            implicit.push(pkg.clone());
        }
    }

    let stack_root = project_root.join(".stack-work");
    if stack_root.is_dir() {
        let roots = find_haskell_stack_deps(&stack_root, &implicit, &user_imports);
        if !roots.is_empty() {
            debug!("Haskell: {} roots via Stack", roots.len());
            return roots;
        }
    }

    // Source packages fetched via `cabal get` land in a flat `cabal-get/`
    // directory alongside the store. These are pre-extracted tarballs with
    // real `.hs` files. Prefer cabal-get over the store for any package that
    // appears in both — the store often holds only compiled `.a`/`.hi`
    // artifacts with no `.hs` source, making the store root useless for
    // indexing.
    let cabal_get_roots = find_haskell_cabal_get_deps(&implicit, &user_imports);
    let cabal_get_names: std::collections::HashSet<String> =
        cabal_get_roots.iter().map(|r| r.module_path.clone()).collect();

    let store_roots = find_haskell_cabal_deps(&implicit, &user_imports);
    // Merge: cabal-get wins on name collision; store fills the rest.
    let mut roots: Vec<ExternalDepRoot> = cabal_get_roots;
    for r in store_roots {
        if !cabal_get_names.contains(&r.module_path) {
            roots.push(r);
        }
    }

    debug!(
        "Haskell: {} roots via Cabal ({} declared, {} GHC-registered)",
        roots.len(),
        declared.len(),
        toolchain_pkgs.len(),
    );
    roots
}

/// Read every package GHC has registered. The `.conf` files under
/// `<ghc-libdir>/package.conf.d/` are the canonical source — each one
/// declares a `name:` field. Parsing the files (rather than the filename
/// `<pkg>-<version>-<hash>.conf`) is robust to dashes in package names.
fn discover_ghc_registered_packages() -> Vec<String> {
    let Some(libdir) = ghc_libdir() else { return Vec::new() };
    let conf_d = libdir.join("package.conf.d");
    let Ok(entries) = std::fs::read_dir(&conf_d) else { return Vec::new() };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) != Some("conf") { continue }
        let Ok(content) = std::fs::read_to_string(&path) else { continue };
        for line in content.lines() {
            let trimmed = line.trim_start();
            // Cabal-style `.conf` files start with `name: <pkg>` (lowercase
            // key, optional whitespace before colon).
            let name = trimmed
                .strip_prefix("name:")
                .or_else(|| trimmed.strip_prefix("name :"));
            if let Some(name) = name {
                let name = name.trim();
                if !name.is_empty() {
                    names.push(name.to_string());
                }
                break;
            }
        }
    }
    names
}

fn ghc_libdir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_GHC_LIBDIR") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p); }
    }
    use std::process::Command;
    let probe = |program: &str| -> Option<PathBuf> {
        let out = Command::new(program).arg("--print-libdir").output().ok()?;
        if !out.status.success() { return None; }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() { return None; }
        let p = PathBuf::from(s);
        if p.is_dir() { Some(p) } else { None }
    };
    if let Some(p) = probe("ghc") { return Some(p); }
    // GHC's Windows shim is `.bat`; std::process::Command doesn't apply
    // PATHEXT so try the explicit name.
    #[cfg(windows)]
    {
        if let Some(p) = probe("ghc.bat") { return Some(p); }
    }
    None
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
    let mut stores: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        stores.push(home.join(".cabal").join("store"));
        stores.push(home.join(".local").join("state").join("cabal").join("store"));
    }
    // Cabal 3.x on Windows defaults to %LOCALAPPDATA%\cabal\store.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        stores.push(PathBuf::from(local).join("cabal").join("store"));
    }
    if let Some(data) = dirs::data_local_dir() {
        stores.push(data.join("cabal").join("store"));
    }
    for store in stores {
        if store.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&store) {
                for e in entries.flatten() {
                    if e.path().is_dir() { candidates.push(e.path()) }
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

/// Discover source packages under the `cabal-get/` directory. `cabal get
/// <pkg>` extracts a tarball to `<cabal_data_dir>/cabal-get/<pkg>-<ver>/`
/// with a `src/` subdirectory holding the actual `.hs` files.
///
/// Also performs one level of transitive discovery: for each found package,
/// reads its `.cabal` file to find dependencies that are also present in the
/// same `cabal-get/` directory. This closes the re-export gap where a package
/// like `hspec` re-exports everything from `hspec-core` — both must be indexed
/// for bare-name symbol lookup to land on the actual definitions.
fn find_haskell_cabal_get_deps(
    declared: &[String],
    user_imports: &[String],
) -> Vec<ExternalDepRoot> {
    let mut get_dirs: Vec<PathBuf> = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        get_dirs.push(PathBuf::from(local).join("cabal").join("cabal-get"));
    }
    if let Some(home) = dirs::home_dir() {
        get_dirs.push(home.join(".cabal").join("cabal-get"));
    }
    if let Some(data) = dirs::data_local_dir() {
        get_dirs.push(data.join("cabal").join("cabal-get"));
    }

    let mut roots = Vec::new();
    for get_dir in get_dirs {
        if !get_dir.is_dir() { continue }
        let direct = find_haskell_cabal_get_deps_in_dir(&get_dir, declared, user_imports);

        // One-level transitive expansion: read each found package's own .cabal
        // to discover sibling packages in the same cabal-get directory. This
        // handles re-export chains (hspec → hspec-core, pandoc → pandoc-types).
        let mut transitive_names: Vec<String> = Vec::new();
        let already_found: std::collections::HashSet<String> =
            direct.iter().map(|r| r.module_path.clone()).collect();
        for root in &direct {
            let pkg_dir = {
                // The root may be `<pkg>-<ver>/src/` — step up to the package dir.
                if root.root.file_name().and_then(|n| n.to_str()) == Some("src") {
                    root.root.parent().unwrap_or(&root.root).to_path_buf()
                } else {
                    root.root.clone()
                }
            };
            let extra = cabal_file_deps_in_dir(&pkg_dir);
            for dep in extra {
                if !already_found.contains(&dep) && !declared.contains(&dep) {
                    transitive_names.push(dep);
                }
            }
        }
        transitive_names.sort();
        transitive_names.dedup();

        let transitive = find_haskell_cabal_get_deps_in_dir(&get_dir, &transitive_names, user_imports);

        roots.extend(direct);
        roots.extend(transitive);
    }
    roots
}

/// Read the `build-depends` of the first `.cabal` file found in `pkg_dir`.
/// Returns package names (no version constraints). Used for one-level
/// transitive dependency discovery within `cabal-get/`.
fn cabal_file_deps_in_dir(pkg_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(pkg_dir) else { return Vec::new() };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) == Some("cabal") {
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            return parse_dep_names_from_cabal_content(&content);
        }
    }
    Vec::new()
}

fn parse_dep_names_from_cabal_content(content: &str) -> Vec<String> {
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

/// Search one `cabal-get/`-style directory for extracted source packages.
/// Extracted as a separate function so tests can supply a controlled directory.
pub(crate) fn find_haskell_cabal_get_deps_in_dir(
    get_dir: &Path,
    declared: &[String],
    user_imports: &[String],
) -> Vec<ExternalDepRoot> {
    let mut roots = Vec::new();
    let Ok(entries) = std::fs::read_dir(get_dir) else { return roots };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        for dep in declared {
            let prefix = format!("{dep}-");
            if name_str.starts_with(&prefix) {
                // The source root is `src/` when present, otherwise the
                // package dir itself (some packages use a flat layout).
                let src = path.join("src");
                let root = if src.is_dir() { src } else { path.clone() };
                let version = name_str[prefix.len()..].to_string();
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version,
                    root,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: user_imports.to_vec(),
                });
                break;
            }
        }
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

/// GHC boot libraries whose file paths don't align with the module names
/// users import. `base` re-exports everything from `ghc-internal` using
/// `reexported-modules`; user code writes `import Data.Functor` but the
/// definition lives at `GHC/Internal/Data/Functor.hs`. A tail-match on
/// `Data/Functor.hs` will never hit `GHC/Internal/Data/Functor.hs`, so
/// narrowing produces an empty set for the file that actually holds the
/// operators. Walk these packages in full.
const GHC_BOOT_PACKAGES: &[&str] = &["ghc-internal", "ghc-prim", "ghc-bignum", "rts"];

fn walk_haskell_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    // GHC boot libs use a path layout that doesn't match user import names.
    if GHC_BOOT_PACKAGES.contains(&dep.module_path.as_str()) {
        return walk_haskell_root(dep);
    }
    if dep.requested_imports.is_empty() { return walk_haskell_root(dep); }
    let tails: std::collections::HashSet<String> = dep
        .requested_imports
        .iter()
        .filter_map(|m| haskell_module_to_path_tail(m))
        .collect();
    if tails.is_empty() { return walk_haskell_root(dep); }

    let mut out = Vec::new();
    walk_haskell_narrowed_dir(&dep.root, &dep.root, dep, &tails, &mut out, 0);
    // If narrowing found nothing — likely a package where module names
    // don't correspond directly to file paths — fall back to full walk.
    if out.is_empty() { return walk_haskell_root(dep); }
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
            // External library sources may legitimately be named `Spec.hs` or
            // `Test.hs` — do not filter them here.
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
            // Do not filter `Spec.hs` or `Test.hs` here — external library
            // sources legitimately use those names (e.g. `hspec-core`'s
            // `Test/Hspec/Core/Spec.hs` defines `it` and `describe`). The
            // user-project walk has its own exclusion logic.
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
    // Collect (package_name, haskell_module_name, walked_file) triples.
    // Haskell module name is derived by treating the path relative to the
    // package root as a dotted identifier: `Text/Hspec/Core/Spec.hs` →
    // `Test.Hspec.Core.Spec`. Both the package name and the module name are
    // inserted as keys so that:
    //   * `locate("hspec-core", "it")` works when re-export chains are not
    //     followed (package-key lookup).
    //   * `locate("Test.Hspec.Core.Spec", "it")` works when an external
    //     file (e.g. `Test/Hspec.hs`) is demand-parsed and its `import
    //     Test.Hspec.Core.Spec` line is turned into an `Imports` ref that
    //     the demand BFS resolves.
    let mut work: Vec<(String, String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_haskell_root(dep) {
            let haskell_module = path_to_haskell_module(&wf.absolute_path, &dep.root);
            work.push((dep.module_path.clone(), haskell_module, wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }
    let per_file: Vec<Vec<(String, String, String, PathBuf)>> = work
        .par_iter()
        .map(|(pkg, haskell_mod, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_haskell_header(&src)
                .into_iter()
                .map(|name| (pkg.clone(), haskell_mod.clone(), name, wf.absolute_path.clone()))
                .collect()
        })
        .collect();
    let mut index = SymbolLocationIndex::new();
    for batch in per_file {
        for (pkg, haskell_mod, name, file) in batch {
            index.insert(pkg, name.clone(), file.clone());
            // Also index under the Haskell module name so that demand-BFS
            // can follow `import Test.Hspec.Core.Spec` to the right file.
            if !haskell_mod.is_empty() {
                index.insert(haskell_mod, name, file);
            }
        }
    }
    index
}

/// Derive the Haskell module name from a file path relative to its package
/// root. `Test/Hspec/Core/Spec.hs` under root `…/hspec-core/src/`
/// becomes `Test.Hspec.Core.Spec`. Returns an empty string when the path
/// cannot be relativised or doesn't end with `.hs`.
pub(crate) fn path_to_haskell_module(file: &Path, root: &Path) -> String {
    let rel = match file.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };
    let s = rel.to_string_lossy();
    let without_ext = s.strip_suffix(".hs").unwrap_or(&s);
    without_ext.replace(['/', '\\'], ".")
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

#[cfg(test)]
#[path = "cabal_tests.rs"]
mod tests;
