// =============================================================================
// indexer/stage_discover.rs — Stage 1: project + package discovery
//
// Three-stage pipeline split (see stage_link.rs for Stage 2). This module
// holds the helpers `full_index` uses during Stage 1:
//
//   * Language-breakdown audit log (`log_language_breakdown`)
//   * Manifest-kind → ecosystem-tag map (`manifest_kind_to_ecosystem`)
//   * Per-package dep-row collection (`collect_package_dep_rows`)
//   * Workspace / monorepo package detection
//     (`detect_packages`, `scan_workspace_dirs`, `package_name_from_manifest`,
//     `dir_name`, `find_manifest_path_abs`, `find_csproj`, `manifest_to_kind`)
//   * Service marking based on Dockerfile presence (`mark_service_packages`)
//
// None of this logic touches tree-sitter or the symbol index — it's pure
// filesystem + manifest inspection. Split out so the driver in `full.rs`
// stays focused on orchestrating the three stages in sequence.
// =============================================================================

use std::path::Path;

use tracing::{debug, info, warn};

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
            *host_symbols.entry(pf.language.clone()).or_insert(0) +=
                pf.symbols.len() as u32;
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

/// Map a `ManifestKind` to the locator ecosystem string used by
/// `ExternalSourceLocator::ecosystem`. The two enums are intentionally
/// separate — manifests are classified by file format (package.json,
/// pyproject.toml) while locators are classified by tool ecosystem — so
/// they must stay linked by an explicit table.
///
/// Unmatched kinds return `None`; the caller skips writing package_deps
/// rows for manifests whose ecosystem has no locator today.
pub(crate) fn manifest_kind_to_ecosystem(
    kind: crate::ecosystem::manifest::ManifestKind,
) -> Option<&'static str> {
    use crate::ecosystem::manifest::ManifestKind;
    Some(match kind {
        ManifestKind::Npm => "typescript",
        ManifestKind::Cargo => "rust",
        ManifestKind::NuGet => "dotnet",
        ManifestKind::GoMod => "go",
        ManifestKind::PyProject => "python",
        ManifestKind::Gradle => "java",
        ManifestKind::Maven => "java",
        ManifestKind::Gemfile => "ruby",
        ManifestKind::Composer => "php",
        ManifestKind::SwiftPM => "swift",
        ManifestKind::Pubspec => "dart",
        ManifestKind::Mix => "elixir",
        ManifestKind::Description => "r",
        ManifestKind::Sbt => "scala",
        ManifestKind::Opam => "ocaml",
        #[allow(unreachable_patterns)]
        _ => return None,
    })
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
            let Some(ecosystem) = manifest_kind_to_ecosystem(kind) else { continue };
            for dep in &data.dependencies {
                rows.push((pkg_id, ecosystem, dep.clone(), None, "runtime"));
            }
            // .NET <ProjectReference> — sibling workspace project intent.
            // Emitted with a distinct kind so downstream consumers can
            // separate external NuGet deps from internal workspace refs.
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
///    handles named workspace systems (Cargo workspace, npm/pnpm workspaces,
///    Turborepo, Nx, Lerna) — these expose authoritative declared names
///    through their workspace manifest.
///
/// 2. **Recursive manifest scan** (`scan_all_manifests`) walks the tree
///    looking for every known per-ecosystem manifest (package.json,
///    pubspec.yaml, Cargo.toml, go.mod, pyproject.toml, mix.exs,
///    Package.swift, composer.json, Gemfile, pom.xml, build.gradle,
///    .csproj/.fsproj/.vbproj, gleam.toml, build.sbt). Polyglot monorepos
///    (Dart in `mobile/`, Swift in `ios/`, Rust in `src-tauri/`) carry
///    siblings the workspace manifest never names — this scan finds them.
///
/// Dedup is by `(path, kind)`. Workspace-source rows are inserted first so
/// they win on conflict (their `declared_name` is more reliable). Same path
/// with different kind always coexists (e.g., a Tauri root with both
/// `Cargo.toml` and `package.json`).
pub(crate) fn detect_packages(project_root: &Path) -> (Vec<PackageInfo>, Option<String>) {
    let mut packages: Vec<PackageInfo> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut workspace_kind: Option<String> = None;

    // 1. Workspace-aware detection (named monorepo systems).
    if let Some(mono) = bearwisdom_profile::scanner::monorepo::detect_monorepo(project_root) {
        let kind_hint = match mono.kind.as_str() {
            "cargo-workspace" => "cargo",
            "npm-workspaces" | "pnpm-workspace" | "turborepo" | "lerna" => "npm",
            "nx" => "npm",
            other => other,
        };

        let mut ws_packages: Vec<PackageInfo> = Vec::new();

        if mono.packages.is_empty() {
            // Profile detected a monorepo kind but no explicit package list.
            // Scan common workspace directories (packages/, apps/, libs/, crates/).
            ws_packages = scan_workspace_dirs(project_root, kind_hint);
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
                                if sub_name.starts_with('.') { continue; }
                                let full_rel = format!("{}/{}", base, sub_name);
                                let abs = project_root.join(&full_rel);
                                let declared_name = package_name_from_manifest(&abs, kind_hint);
                                ws_packages.push(PackageInfo {
                                    id: None,
                                    name: sub_name.clone(),
                                    path: full_rel.replace('\\', "/"),
                                    kind: Some(kind_hint.to_string()),
                                    manifest: find_manifest_path_abs(&abs, kind_hint),
                                    declared_name,
                                });
                            }
                        }
                    }
                } else {
                    let abs = project_root.join(rel_path);
                    if !abs.is_dir() { continue; }
                    let declared_name = package_name_from_manifest(&abs, kind_hint);
                    ws_packages.push(PackageInfo {
                        id: None,
                        name: dir_name(rel_path),
                        path: rel_path.replace('\\', "/"),
                        kind: Some(kind_hint.to_string()),
                        manifest: find_manifest_path_abs(&abs, kind_hint),
                        declared_name,
                    });
                }
            }
        }

        for pkg in ws_packages {
            let key = (pkg.path.clone(), pkg.kind.clone().unwrap_or_default());
            if seen.insert(key) { packages.push(pkg); }
        }

        if !packages.is_empty() {
            workspace_kind = Some(mono.kind);
        }
    }

    // 2. Always run a recursive manifest scan. Picks up sibling ecosystems
    //    that workspace manifests never name (Dart subprojects, iOS, etc.)
    //    plus filling in when no workspace system is detected at all.
    for pkg in scan_all_manifests(project_root) {
        let key = (pkg.path.clone(), pkg.kind.clone().unwrap_or_default());
        if seen.insert(key) { packages.push(pkg); }
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
    walk_for_manifests(project_root, project_root, 0, MAX_DEPTH, &scan_config, &mut out);
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
}

impl ScanConfig {
    fn from_registry(reg: &crate::ecosystem::EcosystemRegistry) -> Self {
        let mut files: Vec<(&'static str, &'static str)> = Vec::new();
        let mut extensions: Vec<(&'static str, &'static str)> = Vec::new();
        let mut pruned: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
        // Universal prune set — VCS metadata is owned by no ecosystem.
        for vcs in &[".git", ".hg", ".svn"] { pruned.insert(*vcs); }
        for eco in reg.all() {
            files.extend(eco.workspace_package_files().iter().copied());
            extensions.extend(eco.workspace_package_extensions().iter().copied());
            for d in eco.pruned_dir_names() { pruned.insert(*d); }
        }
        Self { files, extensions, pruned }
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
    if depth > max_depth { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut subdirs: Vec<std::path::PathBuf> = Vec::new();
    let mut filenames: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let raw_name = entry.file_name();
        let name = raw_name.to_string_lossy().into_owned();
        if file_type.is_dir() {
            // Skip every dotted directory at any depth (`.git`,
            // `.dart_tool`, `.idea`, `.venv`, `.vscode`, ...). Catches the
            // common cases without enumeration; ecosystems still list
            // their non-dotted caches (`node_modules`, `target`, `vendor`,
            // ...) explicitly.
            if name.starts_with('.') && name != "." && name != ".." { continue; }
            if cfg.pruned.contains(name.as_str()) { continue; }
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
    // Extension match — `<name>.csproj`, `<name>.cabal`, etc. One
    // PackageInfo per matched file (each project file is its own package).
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
        rel_dir
            .rsplit('/')
            .next()
            .unwrap_or(&rel_dir)
            .to_string()
    };
    let manifest_rel = if rel_dir.is_empty() {
        manifest_filename.to_string()
    } else {
        format!("{}/{}", rel_dir, manifest_filename)
    };
    let declared_name = package_name_from_manifest(pkg_dir, kind);
    out.push(PackageInfo {
        id: None,
        name: folder_name,
        path: rel_dir,
        kind: Some(kind.to_string()),
        manifest: Some(manifest_rel),
        declared_name,
    });
}

/// Scan common workspace directory patterns (packages/, apps/, libs/, crates/, etc.)
/// for sub-packages containing manifest files.
fn scan_workspace_dirs(project_root: &Path, kind_hint: &str) -> Vec<PackageInfo> {
    let workspace_dirs = ["packages", "apps", "libs", "crates", "modules",
                          "services", "plugins", "integrations", "examples", "src"];
    let manifest_names: &[&str] = &[
        "package.json", "Cargo.toml", "go.mod", "pyproject.toml",
        "pubspec.yaml", "mix.exs", "Package.swift", "composer.json",
    ];
    let mut packages = Vec::new();

    for ws_dir in &workspace_dirs {
        let base = project_root.join(ws_dir);
        if !base.is_dir() { continue; }
        if let Ok(entries) = std::fs::read_dir(&base) {
            for entry in entries.flatten() {
                if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let sub_name = entry.file_name().to_string_lossy().into_owned();
                if sub_name.starts_with('.') || sub_name == "node_modules" {
                    continue;
                }
                let sub = entry.path();
                // Check standard manifest files.
                let mut found = false;
                for mf in manifest_names {
                    if sub.join(mf).exists() {
                        let rel = format!("{}/{}", ws_dir, sub_name);
                        let kind = if kind_hint != "unknown" { kind_hint } else { manifest_to_kind(mf) };
                        let declared_name = package_name_from_manifest(&sub, kind);
                        packages.push(PackageInfo {
                            id: None,
                            name: sub_name.clone(),
                            path: rel,
                            kind: Some(kind.to_string()),
                            manifest: Some(format!("{}/{}/{}", ws_dir, sub_name, mf)),
                            declared_name,
                        });
                        found = true;
                        break;
                    }
                }
                // Check for .csproj (one per directory is a package).
                if !found {
                    if let Some(csproj) = find_csproj(&sub) {
                        let rel = format!("{}/{}", ws_dir, sub_name);
                        // For .NET, the declared name is the .csproj stem.
                        let declared_name = csproj
                            .strip_suffix(".csproj")
                            .map(|s| s.to_string());
                        packages.push(PackageInfo {
                            id: None,
                            name: sub_name.clone(),
                            path: rel,
                            kind: Some("dotnet".to_string()),
                            manifest: Some(format!("{}/{}/{}", ws_dir, sub_name, csproj)),
                            declared_name,
                        });
                    }
                }
            }
        }
    }

    // Also check root-level subdirectories (for .NET solutions, Go multi-module).
    if let Ok(entries) = std::fs::read_dir(project_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let dir_name_str = entry.file_name().to_string_lossy().into_owned();
            if dir_name_str.starts_with('.')
                || dir_name_str == "node_modules"
                || dir_name_str == "target"
                || workspace_dirs.contains(&dir_name_str.as_str())
            {
                continue;
            }
            let sub = entry.path();
            for mf in manifest_names {
                if sub.join(mf).exists() {
                    let kind = if kind_hint != "unknown" { kind_hint } else { manifest_to_kind(mf) };
                    let declared_name = package_name_from_manifest(&sub, kind);
                    // Avoid duplicates from workspace_dirs scan.
                    if !packages.iter().any(|p| p.path == dir_name_str) {
                        packages.push(PackageInfo {
                            id: None,
                            name: dir_name_str.clone(),
                            path: dir_name_str.clone(),
                            kind: Some(kind.to_string()),
                            manifest: Some(format!("{}/{}", dir_name_str, mf)),
                            declared_name,
                        });
                    }
                    break;
                }
            }
        }
    }

    packages
}

/// Try to extract the native package name from a manifest file — the name
/// by which this package is imported by siblings (`@myorg/utils`,
/// `my-crate`, `github.com/user/proj/module`, `MyApp.Api`, etc.). Stored
/// separately from the folder name on `PackageInfo::declared_name`.
fn package_name_from_manifest(dir: &Path, kind: &str) -> Option<String> {
    match kind {
        "npm" => {
            let content = std::fs::read_to_string(dir.join("package.json")).ok()?;
            let v: serde_json::Value = serde_json::from_str(&content).ok()?;
            v.get("name")?.as_str().map(|s| s.to_string())
        }
        "cargo" => {
            let content = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
            // Simple TOML parse: find `name = "..."` under [package].
            let in_package = content.find("[package]")?;
            content[in_package..]
                .lines()
                .find(|l| l.trim().starts_with("name"))
                .and_then(|l| {
                    let val = l.split('=').nth(1)?.trim().trim_matches('"');
                    Some(val.to_string())
                })
        }
        "go" => {
            let content = std::fs::read_to_string(dir.join("go.mod")).ok()?;
            content.lines().next().and_then(|l| {
                l.strip_prefix("module ").map(|m| m.trim().to_string())
            })
        }
        "python" => {
            // pyproject.toml [project].name or [tool.poetry].name.
            let content = std::fs::read_to_string(dir.join("pyproject.toml")).ok()?;
            for marker in &["[project]", "[tool.poetry]"] {
                if let Some(start) = content.find(marker) {
                    // Scan forward until the next section header.
                    let tail = &content[start + marker.len()..];
                    let section = tail.split("\n[").next().unwrap_or(tail);
                    if let Some(name) = section
                        .lines()
                        .find(|l| l.trim_start().starts_with("name"))
                        .and_then(|l| {
                            let val = l.split('=').nth(1)?.trim().trim_matches('"').trim_matches('\'');
                            (!val.is_empty()).then(|| val.to_string())
                        })
                    {
                        return Some(name);
                    }
                }
            }
            None
        }
        "dotnet" => {
            // .csproj / .fsproj / .vbproj filename stem — this is what
            // <ProjectReference> resolves against in MSBuild.
            std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                for ext in &[".csproj", ".fsproj", ".vbproj"] {
                    if let Some(stem) = name.strip_suffix(ext) {
                        return Some(stem.to_string());
                    }
                }
                None
            })
        }
        _ => None,
    }
}

fn dir_name(rel_path: &str) -> String {
    rel_path.rsplit('/').next()
        .or_else(|| rel_path.rsplit('\\').next())
        .unwrap_or(rel_path)
        .to_string()
}

fn find_manifest_path_abs(abs_dir: &Path, kind: &str) -> Option<String> {
    let candidates: &[&str] = match kind {
        "cargo" => &["Cargo.toml"],
        "npm" => &["package.json"],
        "go" => &["go.mod"],
        "python" => &["pyproject.toml"],
        "dart" => &["pubspec.yaml"],
        "elixir" => &["mix.exs"],
        _ => &["package.json", "Cargo.toml", "go.mod", "pyproject.toml"],
    };
    for c in candidates {
        if abs_dir.join(c).exists() {
            // Return path relative to workspace root — caller uses the rel path.
            return Some(c.to_string());
        }
    }
    None
}

fn find_csproj(dir: &Path) -> Option<String> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".csproj") { Some(name) } else { None }
    })
}

fn manifest_to_kind(filename: &str) -> &str {
    match filename {
        "package.json" => "npm",
        "Cargo.toml" => "cargo",
        "go.mod" => "go",
        "pyproject.toml" => "python",
        "pubspec.yaml" => "dart",
        "mix.exs" => "elixir",
        "Package.swift" => "swift",
        "composer.json" => "php",
        _ => "unknown",
    }
}

// ---------------------------------------------------------------------------
// Service package marking
// ---------------------------------------------------------------------------

/// Set `is_service = 1` on packages whose path matches a detected Dockerfile.
///
/// `pairs` is `(package_relative_path, dockerfile_relative_path)` as returned
/// by `crate::languages::dockerfile::connectors::detect_dockerfiles`.
pub(crate) fn mark_service_packages(
    conn: &rusqlite::Connection,
    pairs: &[(String, String)],
) {
    for (pkg_path, dockerfile_path) in pairs {
        match conn.execute(
            "UPDATE packages SET is_service = 1 WHERE path = ?1",
            rusqlite::params![pkg_path],
        ) {
            Ok(n) if n > 0 => {
                debug!("Marked package '{}' as service ({})", pkg_path, dockerfile_path);
            }
            Ok(_) => {
                // Package path not found — may have been cleaned up; not an error.
                debug!("No package row for path '{}' — skipping is_service mark", pkg_path);
            }
            Err(e) => {
                warn!("Failed to mark package '{}' as service: {e}", pkg_path);
            }
        }
    }
}
