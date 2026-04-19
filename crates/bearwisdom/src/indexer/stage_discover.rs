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
/// Uses bearwisdom-profile's monorepo detection first (Cargo workspace,
/// npm workspaces, Turborepo, Nx, Lerna), then falls back to scanning
/// for manifest files in immediate subdirectories.
pub(crate) fn detect_packages(project_root: &Path) -> (Vec<PackageInfo>, Option<String>) {
    // 1. Try bearwisdom-profile monorepo detection.
    if let Some(mono) = bearwisdom_profile::scanner::monorepo::detect_monorepo(project_root) {
        let workspace_kind = mono.kind.clone();
        let kind_hint = match mono.kind.as_str() {
            "cargo-workspace" => "cargo",
            "npm-workspaces" | "pnpm-workspace" | "turborepo" | "lerna" => "npm",
            "nx" => "npm",
            other => other,
        };

        let mut packages: Vec<PackageInfo> = Vec::new();

        if mono.packages.is_empty() {
            // Profile detected a monorepo kind but no explicit package list.
            // Scan common workspace directories (packages/, apps/, libs/, crates/).
            packages = scan_workspace_dirs(project_root, kind_hint);
        } else {
            // Profile returned explicit package paths — these may be globs or
            // directory names. Resolve each to a PackageInfo.
            for rel_path in &mono.packages {
                // Handle glob patterns like "crates/*" from Cargo workspace members.
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
                                packages.push(PackageInfo {
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
                    packages.push(PackageInfo {
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

        if !packages.is_empty() {
            info!("Monorepo detected ({}) — {} packages", workspace_kind, packages.len());
            return (packages, Some(workspace_kind));
        }
    }

    // 2. Fallback: scan workspace-style directories.
    let packages = scan_workspace_dirs(project_root, "unknown");
    if packages.len() >= 2 {
        info!("Fallback package scan — {} packages", packages.len());
        (packages, None)
    } else {
        (Vec::new(), None)
    }
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
