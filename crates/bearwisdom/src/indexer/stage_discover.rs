// =============================================================================
// indexer/stage_discover.rs — Stage 1: project + package discovery
//
// Three-stage pipeline split (see stage_link.rs for Stage 2). This module
// holds the helpers `full_index` uses during Stage 1:
//
//   * Language-breakdown audit log (`log_language_breakdown`)
//   * Registry-owned manifest → dependency-tag lookup
//   * Per-package dep-row collection (`collect_package_dep_rows`)
//   * Workspace / monorepo package detection
//     (`detect_packages`, `scan_workspace_dirs`, registry package metadata)
//
// None of this logic touches tree-sitter or the symbol index — it's pure
// filesystem + manifest inspection. Split out so the driver in `full.rs`
// stays focused on orchestrating the three stages in sequence.
// =============================================================================

use std::path::Path;

use tracing::info;

use crate::types::{PackageInfo, ParsedFile};

// ---------------------------------------------------------------------------
// Language-breakdown audit log
// ---------------------------------------------------------------------------

/// Per-language breakdown of parsed files. Reports host file counts, host
/// symbol counts, and (separately) symbols produced by embedded
/// sub-extractors on each sub-language. A Razor `.cshtml`-heavy project
/// should show e.g. `razor: 120 files, 200 host symbols` alongside
/// `csharp (embedded): 4500 symbols`.
pub(crate) fn log_language_breakdown(parsed: &[ParsedFile]) {
    use std::collections::BTreeMap;

    let mut host_files: BTreeMap<String, u32> = BTreeMap::new();
    let mut host_symbols: BTreeMap<String, u32> = BTreeMap::new();
    let mut embedded_symbols: BTreeMap<String, u32> = BTreeMap::new();

    for pf in parsed {
        *host_files.entry(pf.language.clone()).or_insert(0) += 1;
        // Count symbols by their ACTUAL origin (host language when None, sub
        // extractor language when Some). `symbol_origin_languages` is either
        // empty (all host) or the same length as symbols.
        if pf.symbol_origin_languages.is_empty() {
            *host_symbols.entry(pf.language.clone()).or_insert(0) += pf.symbols.len() as u32;
        } else {
            for origin in &pf.symbol_origin_languages {
                match origin {
                    None => {
                        *host_symbols.entry(pf.language.clone()).or_insert(0) += 1;
                    }
                    Some(sub) => {
                        *embedded_symbols.entry(sub.clone()).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    let detected = crate::languages::LanguageRegistry::detected_languages(parsed);
    info!(
        "Language audit: {} distinct languages ({} host + embedded)",
        detected.len(),
        detected.len()
    );
    for (lang, files) in &host_files {
        let syms = host_symbols.get(lang).copied().unwrap_or(0);
        info!("  {lang}: {files} files, {syms} host symbols");
    }
    for (sub, syms) in &embedded_symbols {
        info!("  {sub} (embedded): {syms} symbols");
    }
}

// ---------------------------------------------------------------------------
// Manifest → ecosystem mapping + dep-row collection
// ---------------------------------------------------------------------------

/// Resolve a normalized manifest kind to the persisted dependency ecosystem
/// tag through the ecosystem registry. Unowned kinds return `None`.
pub(crate) fn manifest_kind_to_ecosystem(
    kind: crate::ecosystem::manifest::ManifestKind,
) -> Option<&'static str> {
    crate::ecosystem::default_registry().package_dependency_ecosystem(kind)
}

/// Collect `(package_id, ecosystem, dep_name, version, kind)` rows for every
/// dependency declared by every workspace package. Sourced from the
/// per-package manifest map already present in `ProjectContext`.
///
/// Returned tuples are ready to pass to `write::write_package_deps`.
/// Version strings are currently None — the manifest readers normalize to
/// a bare dep-name set and drop version specifiers.
pub(crate) fn collect_package_dep_rows(
    ctx: &super::project_context::ProjectContext,
) -> Vec<(i64, &'static str, String, Option<String>, &'static str)> {
    let mut rows = Vec::new();
    for (&pkg_id, manifests) in &ctx.by_package {
        for (&kind, data) in manifests {
            let Some(ecosystem) = manifest_kind_to_ecosystem(kind) else {
                continue;
            };
            for dep in &data.dependencies {
                rows.push((pkg_id, ecosystem, dep.clone(), None, "runtime"));
            }
            // Adapter-normalized sibling package intent is retained as a
            // distinct edge kind for downstream consumers.
            for pr in &data.project_refs {
                rows.push((pkg_id, ecosystem, pr.clone(), None, "project_reference"));
            }
        }
    }
    rows
}

// ---------------------------------------------------------------------------
// Workspace / monorepo package detection
// ---------------------------------------------------------------------------

/// Detect workspace packages. Returns `(packages, workspace_kind)`.
///
/// Two sources are unioned:
///
/// 1. **Workspace-aware detection** via `bearwisdom_profile::scanner::monorepo`
///    handles named workspace systems. Ecosystem adapters normalize the
///    scanner's kind and supply package metadata.
///
/// 2. **Recursive manifest scan** (`scan_all_manifests`) walks every
///    registry-declared package marker. Polyglot repositories can therefore
///    contain siblings that the workspace controller never names.
///
/// Dedup is by `(path, kind)`. Workspace-source rows are inserted first so
/// they win on conflict (their `declared_name` is more reliable). Same path
/// with different kinds always coexists.
pub(crate) fn detect_packages(project_root: &Path) -> (Vec<PackageInfo>, Option<String>) {
    let registry = crate::ecosystem::default_registry();
    let scan_config = ScanConfig::from_registry(registry);
    let mut packages: Vec<PackageInfo> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut workspace_kind: Option<String> = None;

    // 1. Workspace-aware detection (named monorepo systems).
    if let Some(mono) = bearwisdom_profile::scanner::monorepo::detect_monorepo(project_root) {
        let kind_hint = registry
            .workspace_kind_for_monorepo_kind(&mono.kind)
            .unwrap_or(mono.kind.as_str());

        let mut ws_packages: Vec<PackageInfo> = Vec::new();

        if mono.packages.is_empty() {
            // Profile detected a monorepo kind but no explicit package list.
            // Scan common workspace directories (packages/, apps/, libs/, crates/).
            ws_packages = scan_workspace_dirs(project_root, kind_hint, &scan_config);
        } else {
            // Profile returned explicit package paths — these may be globs or
            // directory names. Resolve each to a PackageInfo.
            for rel_path in &mono.packages {
                if rel_path.contains('*') {
                    let base = rel_path.trim_end_matches("/*").trim_end_matches("\\*");
                    let base_dir = project_root.join(base);
                    if base_dir.is_dir() {
                        if let Ok(entries) = std::fs::read_dir(&base_dir) {
                            for entry in entries.flatten() {
                                if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                    continue;
                                }
                                let sub_name = entry.file_name().to_string_lossy().into_owned();
                                if sub_name.starts_with('.') {
                                    continue;
                                }
                                let full_rel = format!("{}/{}", base, sub_name);
                                let abs = project_root.join(&full_rel);
                                let (declared_name, is_publishable) =
                                    read_package_manifest(&abs, kind_hint);
                                ws_packages.push(PackageInfo {
                                    id: None,
                                    name: sub_name.clone(),
                                    path: full_rel.replace('\\', "/"),
                                    kind: Some(kind_hint.to_string()),
                                    manifest: find_manifest_path_abs(&abs, kind_hint),
                                    declared_name,
                                    is_publishable,
                                });
                            }
                        }
                    }
                } else {
                    let abs = project_root.join(rel_path);
                    if !abs.is_dir() {
                        continue;
                    }
                    let (declared_name, is_publishable) = read_package_manifest(&abs, kind_hint);
                    ws_packages.push(PackageInfo {
                        id: None,
                        name: dir_name(rel_path),
                        path: rel_path.replace('\\', "/"),
                        kind: Some(kind_hint.to_string()),
                        manifest: find_manifest_path_abs(&abs, kind_hint),
                        declared_name,
                        is_publishable,
                    });
                }
            }
        }

        for pkg in ws_packages {
            let key = (pkg.path.clone(), pkg.kind.clone().unwrap_or_default());
            if seen.insert(key) {
                packages.push(pkg);
            }
        }

        if !packages.is_empty() {
            workspace_kind = Some(mono.kind);
        }
    }

    // 2. Always run a recursive manifest scan. Picks up sibling ecosystems
    //    that workspace manifests never name.
    //    plus filling in when no workspace system is detected at all.
    //
    //    When a workspace was detected in step 1, the root manifest is
    //    normally the workspace controller, not a member package — including it would
    //    inflate the package table with a synthetic monorepo sibling that
    //    no consumer imports.
    //
    //    Adapter-owned root policy retains hybrid controllers that also
    //    declare a real package.
    let in_workspace = workspace_kind.is_some();
    for pkg in scan_all_manifests(project_root) {
        let is_root_manifest = pkg.path.is_empty() || pkg.path == ".";
        if in_workspace
            && is_root_manifest
            && !workspace_root_has_own_deps(project_root, pkg.kind.as_deref())
        {
            continue;
        }
        let key = (pkg.path.clone(), pkg.kind.clone().unwrap_or_default());
        if seen.insert(key) {
            packages.push(pkg);
        }
    }

    if !packages.is_empty() {
        info!(
            "Workspace detection — {} packages ({})",
            packages.len(),
            workspace_kind.as_deref().unwrap_or("recursive scan only")
        );
    }
    (packages, workspace_kind)
}

/// Recursively walk the project tree looking for every known ecosystem
/// manifest. Bounded depth, prunes dependency caches and build outputs.
/// Multiple manifests in the same directory each register their own
/// PackageInfo (kept distinct downstream by the `(path, kind)` composite
/// key on the `packages` table).
///
/// Markers and prune lists come from the `EcosystemRegistry` — each
/// ecosystem owns the truth about its own manifests and its own
/// dependency cache directories. The orchestrator doesn't carry hardcoded
/// per-ecosystem knowledge.
///
/// Visible to tests via `pub(crate)` so the new fixture-driven tests can
/// exercise the scanner directly.
pub(crate) fn scan_all_manifests(project_root: &Path) -> Vec<PackageInfo> {
    const MAX_DEPTH: u32 = 8;
    let registry = crate::ecosystem::default_registry();
    let scan_config = ScanConfig::from_registry(registry);
    let mut out = Vec::new();
    walk_for_manifests(
        project_root,
        project_root,
        0,
        MAX_DEPTH,
        &scan_config,
        &mut out,
    );
    out
}

/// Pre-flattened scan inputs harvested from the ecosystem registry once
/// per `scan_all_manifests` call. Hot paths (per-directory matching) read
/// from `&[..]` slices instead of dispatching through the trait per file.
struct ScanConfig {
    /// `(filename, kind)` pairs from every ecosystem's
    /// `workspace_package_files()`. Multiple ecosystems may declare the
    /// same `(filename, kind)` — that's fine, the per-directory match
    /// emits one PackageInfo per `(filename, kind)` and the downstream
    /// dedup keys on `(path, kind)`.
    files: Vec<(&'static str, &'static str)>,
    /// `(extension, kind)` pairs from every ecosystem's
    /// `workspace_package_extensions()`. Per-file suffix match.
    extensions: Vec<(&'static str, &'static str)>,
    /// Union of every ecosystem's `pruned_dir_names()` plus the universal
    /// VCS metadata directories (`.git`, `.hg`, `.svn`).
    pruned: std::collections::HashSet<&'static str>,
    /// Conventional workspace member roots supplied by ecosystem adapters.
    member_dirs: Vec<&'static str>,
}

impl ScanConfig {
    fn from_registry(reg: &crate::ecosystem::EcosystemRegistry) -> Self {
        let mut files: Vec<(&'static str, &'static str)> = Vec::new();
        let mut extensions: Vec<(&'static str, &'static str)> = Vec::new();
        let mut pruned: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
        let mut member_dirs: Vec<&'static str> = Vec::new();
        // Universal prune set — VCS metadata is owned by no ecosystem.
        for vcs in &[".git", ".hg", ".svn"] {
            pruned.insert(*vcs);
        }
        for eco in reg.all() {
            files.extend(eco.workspace_package_files().iter().copied());
            extensions.extend(eco.workspace_package_extensions().iter().copied());
            for d in eco.pruned_dir_names() {
                pruned.insert(*d);
            }
            for dir in eco.workspace_member_directories() {
                if !member_dirs.contains(dir) {
                    member_dirs.push(*dir);
                }
            }
        }
        Self {
            files,
            extensions,
            pruned,
            member_dirs,
        }
    }

    fn manifest_in_directory(&self, dir: &Path) -> Option<(String, &'static str)> {
        for (filename, kind) in &self.files {
            if dir.join(filename).is_file() {
                return Some(((*filename).to_owned(), *kind));
            }
        }
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            if !entry.file_type().ok()?.is_file() {
                continue;
            }
            let filename = entry.file_name().to_string_lossy().into_owned();
            if let Some((_, kind)) = self
                .extensions
                .iter()
                .find(|(extension, _)| filename.ends_with(extension))
            {
                return Some((filename, *kind));
            }
        }
        None
    }
}

/// Recursive helper for `scan_all_manifests`. Single allocation-light walk:
/// for each directory, list children once, register every matching manifest,
/// then descend into non-pruned subdirectories.
fn walk_for_manifests(
    project_root: &Path,
    dir: &Path,
    depth: u32,
    max_depth: u32,
    cfg: &ScanConfig,
    out: &mut Vec<PackageInfo>,
) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs: Vec<std::path::PathBuf> = Vec::new();
    let mut filenames: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let raw_name = entry.file_name();
        let name = raw_name.to_string_lossy().into_owned();
        if file_type.is_dir() {
            // Skip every dotted directory at any depth (`.git`,
            // `.dart_tool`, `.idea`, `.venv`, `.vscode`, ...). Catches the
            // common cases without enumeration; ecosystems list their
            // non-dotted caches explicitly.
            if name.starts_with('.') && name != "." && name != ".." {
                continue;
            }
            if cfg.pruned.contains(name.as_str()) {
                continue;
            }
            subdirs.push(entry.path());
        } else if file_type.is_file() {
            filenames.push(name);
        }
    }

    // Exact filename match — registry-driven. Multiple kinds at the same
    // dir are legitimate; the downstream dedup keys on `(path, kind)`.
    for (manifest_name, kind) in &cfg.files {
        if filenames.iter().any(|n| n.as_str() == *manifest_name) {
            register_manifest(project_root, dir, manifest_name, kind, out);
        }
    }
    // Extension markers can carry a package name in their filename. One
    // PackageInfo is emitted per matching file.
    for fname in &filenames {
        for (ext, kind) in &cfg.extensions {
            if fname.ends_with(ext) {
                register_manifest(project_root, dir, fname, kind, out);
            }
        }
    }

    for sub in subdirs {
        walk_for_manifests(project_root, &sub, depth + 1, max_depth, cfg, out);
    }
}

fn register_manifest(
    project_root: &Path,
    pkg_dir: &Path,
    manifest_filename: &str,
    kind: &str,
    out: &mut Vec<PackageInfo>,
) {
    let rel_dir = pkg_dir
        .strip_prefix(project_root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let folder_name = if rel_dir.is_empty() {
        // Root-level manifest. Use the project root's directory name as a
        // friendly label; falls back to "root" if the path is unusual.
        project_root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "root".to_string())
    } else {
        rel_dir.rsplit('/').next().unwrap_or(&rel_dir).to_string()
    };
    let manifest_rel = if rel_dir.is_empty() {
        manifest_filename.to_string()
    } else {
        format!("{}/{}", rel_dir, manifest_filename)
    };
    let (declared_name, is_publishable) = read_package_manifest(pkg_dir, kind);
    out.push(PackageInfo {
        id: None,
        name: folder_name,
        path: rel_dir,
        kind: Some(kind.to_string()),
        manifest: Some(manifest_rel),
        declared_name,
        is_publishable,
    });
}

/// Scan adapter-declared workspace roots for immediate member packages.
fn scan_workspace_dirs(
    project_root: &Path,
    kind_hint: &str,
    config: &ScanConfig,
) -> Vec<PackageInfo> {
    let mut packages = Vec::new();
    let mut scanned = std::collections::HashSet::new();

    for root_name in &config.member_dirs {
        let base = project_root.join(root_name);
        let Ok(entries) = std::fs::read_dir(&base) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || config.pruned.contains(name.as_str()) {
                continue;
            }
            let rel = format!("{root_name}/{name}");
            if scanned.insert(rel.clone()) {
                register_workspace_member(&entry.path(), rel, kind_hint, config, &mut packages);
            }
        }
    }

    if let Ok(entries) = std::fs::read_dir(project_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.')
                || config.pruned.contains(name.as_str())
                || config.member_dirs.contains(&name.as_str())
            {
                continue;
            }
            if scanned.insert(name.clone()) {
                register_workspace_member(&entry.path(), name, kind_hint, config, &mut packages);
            }
        }
    }
    packages
}

fn register_workspace_member(
    dir: &Path,
    path: String,
    kind_hint: &str,
    config: &ScanConfig,
    packages: &mut Vec<PackageInfo>,
) {
    let Some((manifest, inferred_kind)) = config.manifest_in_directory(dir) else {
        return;
    };
    let kind = (kind_hint != "unknown")
        .then_some(kind_hint)
        .unwrap_or(inferred_kind);
    let metadata = crate::ecosystem::default_registry().workspace_package_metadata(dir, kind);
    let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
    packages.push(PackageInfo {
        id: None,
        name,
        manifest: Some(format!("{path}/{manifest}")),
        path,
        kind: Some(kind.to_owned()),
        declared_name: metadata.declared_name,
        is_publishable: metadata.is_publishable,
    });
}

/// Try to extract the native package name from a manifest file — the name
/// by which this package is imported by siblings (`@myorg/utils`,
/// `my-crate`, `github.com/user/proj/module`, `MyApp.Api`, etc.). Stored
/// separately from the folder name on `PackageInfo::declared_name`.
///
/// Returns `(declared_name, is_publishable)`. `is_publishable = false`
/// signals the dead-code `exported_api` contributor that this package's
/// public surface is workspace-internal — its public symbols should not
/// auto-anchor reachability. Default `true` matches the v0 behavior.
pub(crate) fn read_package_manifest(dir: &Path, kind: &str) -> (Option<String>, bool) {
    let metadata = crate::ecosystem::default_registry().workspace_package_metadata(dir, kind);
    (metadata.declared_name, metadata.is_publishable)
}

/// Ask the owning ecosystem whether this workspace controller also declares a
/// root package. Unknown kinds fail closed and remain controller-only.
fn workspace_root_has_own_deps(project_root: &Path, kind: Option<&str>) -> bool {
    kind.is_some_and(|kind| {
        crate::ecosystem::default_registry().workspace_root_is_package(project_root, kind)
    })
}

fn dir_name(rel_path: &str) -> String {
    rel_path
        .rsplit('/')
        .next()
        .or_else(|| rel_path.rsplit('\\').next())
        .unwrap_or(rel_path)
        .to_string()
}

fn find_manifest_path_abs(abs_dir: &Path, kind: &str) -> Option<String> {
    crate::ecosystem::default_registry().workspace_manifest_filename(abs_dir, kind)
}

#[cfg(test)]
#[path = "stage_discover_tests.rs"]
mod tests;
