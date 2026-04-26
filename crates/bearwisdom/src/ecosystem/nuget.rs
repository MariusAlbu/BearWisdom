// =============================================================================
// ecosystem/nuget.rs — NuGet ecosystem (.NET: C#, F#, VB.NET)
//
// Phase 2 + 3: consolidates `indexer/externals/dotnet.rs` +
// `indexer/manifest/nuget.rs`. .NET externals are metadata-only: DLLs are
// parsed via the `dotscope` ECMA-335 reader and emitted as synthetic
// `ParsedFile` rows — no source walk. The pipeline uses
// `parse_metadata_only()` instead of the usual locate_roots/walk_root path.
//
// Languages: csharp, fsharp, vbnet. All three consume the same DLLs from
// `~/.nuget/packages/`. The file-level `language` tag on emitted parsed
// files follows the owning .csproj/.fsproj/.vbproj file type.
//
// Hybrid source + metadata strategy (additive, no flag flip):
//   The DLL metadata path (`parse_metadata_only`) remains the primary eager
//   pass — `uses_demand_driven_parse` stays `false`. A supplementary source
//   scan runs alongside it: for each resolved package directory we look for
//   `.cs` files under `contentFiles/cs/<tfm>/`, `lib/<tfm>/`, `src/`, and
//   the package root. Any found source files are parsed header-only
//   (top-level namespace/class/interface/enum/struct/method decls) and emitted
//   as additional `ParsedFile` rows. When source and DLL metadata provide the
//   same qname, source wins at query time because it carries real line numbers.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("nuget");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["csharp", "fsharp", "vbnet"];
const LEGACY_ECOSYSTEM_TAG: &str = "dotnet";

pub struct NugetEcosystem;

impl Ecosystem for NugetEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn workspace_package_extensions(&self) -> &'static [(&'static str, &'static str)] {
        // .NET project files embed the project name as the filename stem,
        // so they must be matched by extension. One row per project file.
        &[
            (".csproj", "dotnet"),
            (".fsproj", "dotnet"),
            (".vbproj", "dotnet"),
        ]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        // No `packages/` here — that's a canonical npm-monorepo workspace
        // directory and pruning it would block sibling Dart/iOS/Rust pkgs.
        // NuGet's package cache lives at `~/.nuget/packages/`, not in the
        // repo.
        &["bin", "obj", ".nuget"]
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("csharp"),
            EcosystemActivation::LanguagePresent("fsharp"),
            EcosystemActivation::LanguagePresent("vbnet"),
        ])
    }

    // NuGet is metadata-only: no source dep roots, no walking. Return empty
    // from locate_roots so the pipeline knows there's nothing to walk; the
    // legacy indexer consumes parse_metadata_only() directly below.
    fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        Vec::new()
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    // `uses_demand_driven_parse` intentionally stays `false`.
    //
    // The DLL metadata path is the primary eager pass — flipping this to
    // `true` would disable `parse_metadata_only` and leave the indexer
    // relying only on the demand loop, which requires `locate_roots` to return
    // real dep roots. Since `locate_roots` returns empty (NuGet has no source
    // walk), flipping would cause a complete regression.
    //
    // The new source-index path is SUPPLEMENTARY: it runs inside
    // `parse_metadata_only` alongside the DLL scan, so source wins on
    // qnames it covers while DLL metadata fills the rest. No flag change
    // needed.

    /// Build a supplementary `(module, name) → file` index over any `.cs`
    /// source files found inside NuGet package dirs. Consumed by chain walkers
    /// that need a file path for a specific qname — when source resolves it,
    /// source wins over the DLL-synthesized row.
    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_nuget_source_symbol_index(dep_roots)
    }

    /// Resolve a specific import against the supplementary source index.
    /// Falls back to empty when no source covers the requested symbols.
    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        symbols: &[&str],
    ) -> Vec<WalkedFile> {
        resolve_nuget_source_symbols(dep, symbols)
    }

    /// Resolve a single fully-qualified name from the source index.
    /// Falls back to empty when no source covers `fqn`.
    fn resolve_symbol(&self, dep: &ExternalDepRoot, fqn: &str) -> Vec<WalkedFile> {
        let short = fqn.rsplit('.').next().unwrap_or(fqn);
        resolve_nuget_source_symbols(dep, &[short])
    }
}

impl ExternalSourceLocator for NugetEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn parse_metadata_only(&self, project_root: &Path) -> Option<Vec<crate::types::ParsedFile>> {
        let (mut parsed, source_pf) = parse_dotnet_externals_with_source(project_root);
        parsed.extend(source_pf);
        if parsed.is_empty() { None } else { Some(parsed) }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<NugetEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(NugetEcosystem)).clone()
}

// ===========================================================================
// Manifest reader (migrated from indexer/manifest/nuget.rs)
// ===========================================================================

pub struct NuGetManifest;

impl ManifestReader for NuGetManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::NuGet }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let per_proj = self.read_all(project_root);
        if per_proj.is_empty() { return None }

        let mut data = ManifestData::default();
        let mut sdk_types = Vec::new();

        for entry in &per_proj {
            data.dependencies.extend(entry.data.dependencies.iter().cloned());
            for ns in &entry.data.global_usings {
                if !data.global_usings.contains(ns) { data.global_usings.push(ns.clone()) }
            }
            if let Some(sdk) = entry.data.sdk_type.as_deref().and_then(sdk_from_name) {
                sdk_types.push(sdk);
            }
            for pr in &entry.data.project_refs {
                if !data.project_refs.contains(pr) { data.project_refs.push(pr.clone()) }
            }
        }

        let sdk = most_capable_sdk(&sdk_types);
        data.sdk_type = Some(sdk_type_name(sdk).to_string());
        for ns in implicit_usings_for_sdk(sdk) {
            if !data.global_usings.contains(&ns.to_string()) {
                data.global_usings.push(ns.to_string());
            }
        }
        Some(data)
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let csproj_files = find_csproj_files(project_root);
        let mut out = Vec::new();
        for manifest_path in csproj_files {
            let Ok(content) = std::fs::read_to_string(&manifest_path) else { continue };

            let mut data = ManifestData::default();
            let sdk = parse_sdk_type(&content).unwrap_or(DotnetSdkType::Base);
            data.sdk_type = Some(sdk_type_name(sdk).to_string());

            for pkg in parse_package_references(&content) {
                data.dependencies.insert(pkg);
            }
            data.project_refs = parse_project_references(&content);

            let package_dir = manifest_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| project_root.to_path_buf());

            for ns in implicit_usings_for_sdk(sdk) {
                if !data.global_usings.contains(&ns.to_string()) {
                    data.global_usings.push(ns.to_string());
                }
            }
            for path in find_global_using_files(&package_dir) {
                if let Ok(gu_content) = std::fs::read_to_string(&path) {
                    for ns in parse_global_usings(&gu_content) {
                        if !data.global_usings.contains(&ns) { data.global_usings.push(ns) }
                    }
                }
            }

            let name = manifest_path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned());

            out.push(ReaderEntry { package_dir, manifest_path, data, name });
        }
        out
    }
}

fn sdk_from_name(name: &str) -> Option<DotnetSdkType> {
    Some(match name {
        "base" => DotnetSdkType::Base,
        "web" => DotnetSdkType::Web,
        "worker" => DotnetSdkType::Worker,
        "blazor" => DotnetSdkType::Blazor,
        "other" => DotnetSdkType::Other,
        _ => return None,
    })
}

fn find_csproj_files(root: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    collect_csproj(root, &mut result, 0);
    result
}

fn collect_csproj(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 10 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                "bin" | "obj" | "node_modules" | ".git" | "target"
                    | "packages" | ".vs" | "TestResults" | "artifacts"
            ) { continue }
            collect_csproj(&path, out, depth + 1);
        } else if path.extension().is_some_and(|e| e == "csproj" || e == "fsproj" || e == "vbproj") {
            out.push(path);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DotnetSdkType { Base, Web, Worker, Blazor, Other }

fn sdk_type_name(sdk: DotnetSdkType) -> &'static str {
    match sdk {
        DotnetSdkType::Base => "base",
        DotnetSdkType::Web => "web",
        DotnetSdkType::Worker => "worker",
        DotnetSdkType::Blazor => "blazor",
        DotnetSdkType::Other => "other",
    }
}

pub fn parse_sdk_type(content: &str) -> Option<DotnetSdkType> {
    let sdk_start = content.find("Sdk=\"")?;
    let rest = &content[sdk_start + 5..];
    let sdk_end = rest.find('"')?;
    let sdk_str = &rest[..sdk_end];
    Some(match sdk_str {
        "Microsoft.NET.Sdk" => DotnetSdkType::Base,
        "Microsoft.NET.Sdk.Web" => DotnetSdkType::Web,
        "Microsoft.NET.Sdk.Worker" => DotnetSdkType::Worker,
        "Microsoft.NET.Sdk.BlazorWebAssembly" => DotnetSdkType::Blazor,
        _ => DotnetSdkType::Other,
    })
}

pub fn parse_package_references(content: &str) -> Vec<String> {
    parse_package_references_full(content).into_iter().map(|c| c.name).collect()
}

pub fn parse_project_references(content: &str) -> Vec<String> {
    let tag = "ProjectReference";
    let mut out = Vec::new();
    let mut search_from = 0;
    while let Some(pos) = content[search_from..].find(tag) {
        let abs_pos = search_from + pos;
        search_from = abs_pos + tag.len();
        let rest = &content[search_from..];
        let window = &rest[..rest.len().min(512)];
        let Some(inc_pos) = window.find("Include=\"") else { continue };
        let after_inc = &window[inc_pos + 9..];
        let Some(end) = after_inc.find('"') else { continue };
        let raw = &after_inc[..end];
        if raw.is_empty() { continue }
        let last = raw.rsplit(|c: char| c == '/' || c == '\\').next().unwrap_or(raw);
        let stem = last
            .strip_suffix(".csproj")
            .or_else(|| last.strip_suffix(".fsproj"))
            .or_else(|| last.strip_suffix(".vbproj"))
            .unwrap_or(last);
        if stem.is_empty() { continue }
        let stem = stem.to_string();
        if !out.contains(&stem) { out.push(stem) }
    }
    out
}

#[derive(Debug, Clone)]
pub struct NuGetCoord {
    pub name: String,
    pub version: Option<String>,
}

pub fn parse_package_references_full(content: &str) -> Vec<NuGetCoord> {
    let mut coords = Vec::new();
    let tag = "PackageReference";
    let mut search_from = 0;
    while let Some(pos) = content[search_from..].find(tag) {
        let abs_pos = search_from + pos;
        search_from = abs_pos + tag.len();
        let rest = &content[search_from..];
        let window = &rest[..rest.len().min(256)];
        let name = window.find("Include=\"").and_then(|inc_pos| {
            let after_inc = &window[inc_pos + 9..];
            after_inc.find('"').map(|end| after_inc[..end].to_string()).filter(|s| !s.is_empty())
        });
        let Some(name) = name else { continue };
        let version = window.find("Version=\"").and_then(|ver_pos| {
            let after_ver = &window[ver_pos + 9..];
            after_ver.find('"').map(|end| after_ver[..end].to_string())
                .filter(|v| !v.is_empty() && !v.starts_with("$("))
        });
        coords.push(NuGetCoord { name, version });
    }
    coords
}

pub fn most_capable_sdk(sdks: &[DotnetSdkType]) -> DotnetSdkType {
    if sdks.contains(&DotnetSdkType::Web) { DotnetSdkType::Web }
    else if sdks.contains(&DotnetSdkType::Worker) { DotnetSdkType::Worker }
    else if sdks.contains(&DotnetSdkType::Blazor) { DotnetSdkType::Blazor }
    else if sdks.contains(&DotnetSdkType::Base) { DotnetSdkType::Base }
    else { DotnetSdkType::Other }
}

pub fn implicit_usings_for_sdk(sdk: DotnetSdkType) -> Vec<&'static str> {
    let mut usings = vec![
        "System", "System.Collections.Generic", "System.IO",
        "System.Linq", "System.Net.Http", "System.Threading", "System.Threading.Tasks",
    ];
    match sdk {
        DotnetSdkType::Web => usings.extend_from_slice(&[
            "System.Net.Http.Json",
            "Microsoft.AspNetCore.Builder", "Microsoft.AspNetCore.Hosting",
            "Microsoft.AspNetCore.Http", "Microsoft.AspNetCore.Http.HttpResults",
            "Microsoft.AspNetCore.Mvc", "Microsoft.AspNetCore.Routing",
            "Microsoft.Extensions.Configuration", "Microsoft.Extensions.DependencyInjection",
            "Microsoft.Extensions.Hosting", "Microsoft.Extensions.Logging",
        ]),
        DotnetSdkType::Worker => usings.extend_from_slice(&[
            "Microsoft.Extensions.Configuration", "Microsoft.Extensions.DependencyInjection",
            "Microsoft.Extensions.Hosting", "Microsoft.Extensions.Logging",
        ]),
        DotnetSdkType::Blazor => usings.extend_from_slice(&[
            "System.Net.Http.Json",
            "Microsoft.AspNetCore.Components", "Microsoft.AspNetCore.Components.Forms",
            "Microsoft.AspNetCore.Components.Routing", "Microsoft.AspNetCore.Components.Web",
            "Microsoft.Extensions.Configuration", "Microsoft.Extensions.DependencyInjection",
            "Microsoft.Extensions.Logging",
        ]),
        _ => {}
    }
    usings
}

fn find_global_using_files(root: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    collect_global_usings(root, &mut result, 0);
    result
}

fn collect_global_usings(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 10 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                "bin" | "obj" | "node_modules" | ".git" | "target"
                    | "packages" | ".vs" | "TestResults" | "artifacts"
            ) { continue }
            collect_global_usings(&path, out, depth + 1);
        } else {
            let name = entry.file_name();
            let name_lower = name.to_string_lossy().to_lowercase();
            if name_lower.contains("globalusing") || name_lower == "usings.cs" {
                out.push(path);
            }
        }
    }
}

pub fn parse_global_usings(content: &str) -> Vec<String> {
    let mut usings = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("global using") {
            let rest = rest.trim();
            if rest.starts_with("static ") { continue }
            let ns = rest.trim_end_matches(';').trim();
            if !ns.is_empty() { usings.push(ns.to_string()) }
        }
    }
    usings
}

// ===========================================================================
// NuGet cache + DLL metadata parsing (migrated from externals/dotnet.rs)
// ===========================================================================

/// Public entry point used by back-compat re-exports in `externals.rs`.
/// Returns DLL metadata ParsedFiles only — source ParsedFiles are merged by
/// the `ExternalSourceLocator::parse_metadata_only` impl above.
pub fn parse_dotnet_externals(project_root: &Path) -> Vec<crate::types::ParsedFile> {
    let (dll_pf, _source_pf) = parse_dotnet_externals_with_source(project_root);
    dll_pf
}

/// Internal: returns `(dll_parsed_files, source_parsed_files)`. Called by the
/// `ExternalSourceLocator::parse_metadata_only` impl which concatenates both.
/// Keeping them separate lets back-compat callers stay cheap (DLL-only).
fn parse_dotnet_externals_with_source(
    project_root: &Path,
) -> (Vec<crate::types::ParsedFile>, Vec<crate::types::ParsedFile>) {
    let mut project_files: Vec<PathBuf> = Vec::new();
    collect_dotnet_project_files(project_root, &mut project_files, 0);
    if project_files.is_empty() { return (Vec::new(), Vec::new()) }

    let mut coords: Vec<NuGetCoord> = Vec::new();
    for p in &project_files {
        let Ok(content) = std::fs::read_to_string(p) else { continue };
        coords.extend(parse_package_references_full(&content));
    }

    for p in &project_files {
        if let Some(proj_dir) = p.parent() {
            coords.extend(collect_transitive_coords_from_deps_json(proj_dir));
        }
    }

    if coords.is_empty() { return (Vec::new(), Vec::new()) }

    let Some(nuget_root) = nuget_packages_root() else {
        debug!("No NuGet packages cache; skipping .NET externals");
        return (Vec::new(), Vec::new());
    };
    debug!(
        "Probing NuGet cache {} for {} package references",
        nuget_root.display(),
        coords.len()
    );

    let lang_id = dominant_dotnet_language(&project_files);

    // Per-coord work runs in parallel — DLL metadata reads and per-file
    // source parses are I/O + CPU bound and independent across packages.
    // Dedupe (seen_dll / seen_src) is done single-threaded after the
    // parallel pass so Vec ordering stays deterministic. On a big .NET
    // solution (2000+ transitives) this is the dominant externals cost
    // and a near-linear win per available core.
    struct CoordResult {
        dll: Option<(PathBuf, crate::types::ParsedFile)>,
        srcs: Vec<(PathBuf, crate::types::ParsedFile)>,
    }

    let per_coord: Vec<CoordResult> = coords
        .par_iter()
        .map(|coord| {
            let pkg_dir = nuget_root.join(coord.name.to_lowercase());
            if !pkg_dir.is_dir() {
                return CoordResult { dll: None, srcs: Vec::new() };
            }

            let version = if let Some(v) = &coord.version {
                let concrete = pkg_dir.join(v);
                if concrete.is_dir() { v.clone() }
                else {
                    match largest_version_subdir(&pkg_dir) {
                        Some(v) => v,
                        None => return CoordResult { dll: None, srcs: Vec::new() },
                    }
                }
            } else {
                match largest_version_subdir(&pkg_dir) {
                    Some(v) => v,
                    None => return CoordResult { dll: None, srcs: Vec::new() },
                }
            };
            let version_dir = pkg_dir.join(&version);

            let dll = find_dll_in_version_dir(&version_dir, &coord.name)
                .and_then(|dll_path| match parse_dotnet_dll(&dll_path, &coord.name, lang_id) {
                    Ok(pf) => Some((dll_path, pf)),
                    Err(e) => {
                        debug!("Failed .NET metadata read {}: {e}", dll_path.display());
                        None
                    }
                });

            let mut srcs = Vec::new();
            for src_path in discover_nuget_source_files(&version_dir) {
                match parse_cs_source_file(&src_path, &coord.name, lang_id) {
                    Ok(pf) => srcs.push((src_path, pf)),
                    Err(e) => debug!("NuGet source parse error {}: {e}", src_path.display()),
                }
            }

            CoordResult { dll, srcs }
        })
        .collect();

    let mut dll_out = Vec::new();
    let mut src_out = Vec::new();
    let mut seen_dll: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut seen_src: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for res in per_coord {
        if let Some((path, pf)) = res.dll {
            if seen_dll.insert(path) { dll_out.push(pf); }
        }
        for (path, pf) in res.srcs {
            if seen_src.insert(path) {
                debug!("NuGet source: {} symbols from {}", pf.symbols.len(), pf.path);
                src_out.push(pf);
            }
        }
    }

    if !src_out.is_empty() {
        debug!(
            "NuGet hybrid: {} DLL + {} source-file entries for {}",
            dll_out.len(), src_out.len(), project_root.display()
        );
    }

    (dll_out, src_out)
}

fn collect_transitive_coords_from_deps_json(proj_dir: &Path) -> Vec<NuGetCoord> {
    let mut deps_json_files: Vec<PathBuf> = Vec::new();
    collect_deps_json(&proj_dir.join("bin"), &mut deps_json_files, 0);
    if deps_json_files.is_empty() { return Vec::new() }

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in deps_json_files.iter().take(16) {
        let Ok(content) = std::fs::read_to_string(path) else { continue };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else { continue };
        let Some(libs) = json.get("libraries").and_then(|v| v.as_object()) else { continue };
        for (key, value) in libs {
            let ty = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if ty != "package" { continue }
            let Some((name, version)) = key.rsplit_once('/') else { continue };
            if !seen.insert(key.clone()) { continue }
            out.push(NuGetCoord { name: name.to_string(), version: Some(version.to_string()) });
        }
    }
    out
}

fn collect_deps_json(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 5 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(name, "obj" | "runtimes" | "ref") { continue }
                }
                collect_deps_json(&path, out, depth + 1);
            } else if ft.is_file()
                && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".deps.json"))
            {
                out.push(path);
            }
        }
    }
}

fn dominant_dotnet_language(project_files: &[PathBuf]) -> &'static str {
    let mut cs = 0usize;
    let mut fs = 0usize;
    let mut vb = 0usize;
    for p in project_files {
        match p.extension().and_then(|e| e.to_str()) {
            Some("csproj") => cs += 1,
            Some("fsproj") => fs += 1,
            Some("vbproj") => vb += 1,
            _ => {}
        }
    }
    if cs >= fs && cs >= vb { "csharp" }
    else if fs >= vb { "fsharp" }
    else { "vb" }
}

pub fn nuget_packages_root() -> Option<PathBuf> {
    for key in ["BEARWISDOM_NUGET_PACKAGES", "NUGET_PACKAGES"] {
        if let Some(raw) = std::env::var_os(key) {
            let p = PathBuf::from(raw);
            if p.is_dir() { return Some(p) }
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join(".nuget").join("packages");
    if candidate.is_dir() { Some(candidate) } else { None }
}

/// Locate the `.dll` matching `pkg_name` inside an already-resolved
/// `<nuget-cache>/<pkg-id>/<version>/` directory. Returns `None` for
/// source-only packages that ship no `lib/` directory.
fn find_dll_in_version_dir(version_dir: &Path, pkg_name: &str) -> Option<PathBuf> {
    let lib_dir = version_dir.join("lib");
    if !lib_dir.is_dir() { return None }

    let preferred_tfms = ["net9.0", "net8.0", "net7.0", "net6.0", "netstandard2.1", "netstandard2.0"];
    let mut chosen_tfm: Option<PathBuf> = None;
    for tfm in preferred_tfms {
        let candidate = lib_dir.join(tfm);
        if candidate.is_dir() { chosen_tfm = Some(candidate); break }
    }
    let tfm_dir = chosen_tfm.or_else(|| largest_subdir(&lib_dir))?;

    let entries = std::fs::read_dir(&tfm_dir).ok()?;
    let target_lower = pkg_name.to_lowercase() + ".dll";
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name == target_lower { return Some(entry.path()) }
    }
    None
}

fn largest_version_subdir(dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut versions: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            if e.file_type().ok()?.is_dir() { e.file_name().into_string().ok() } else { None }
        })
        .collect();
    versions.sort();
    versions.into_iter().next_back()
}

fn largest_subdir(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subs: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            if e.file_type().ok()?.is_dir() { Some(e.path()) } else { None }
        })
        .collect();
    subs.sort();
    subs.into_iter().next_back()
}

/// Public shim so the DotnetStdlib ecosystem can reuse this DLL→ParsedFile
/// synthesizer for .NET reference assemblies. Identical contract to the
/// private helper.
pub(crate) fn parse_dotnet_dll_public(
    dll_path: &Path,
    package_name: &str,
    lang_id: &str,
) -> std::result::Result<crate::types::ParsedFile, String> {
    parse_dotnet_dll(dll_path, package_name, lang_id)
}

fn parse_dotnet_dll(
    dll_path: &Path,
    package_name: &str,
    lang_id: &str,
) -> std::result::Result<crate::types::ParsedFile, String> {
    use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind};
    use dotscope::metadata::method::MethodAccessFlags;
    use dotscope::prelude::CilObject;

    let assembly = CilObject::from_path(dll_path).map_err(|e| e.to_string())?;
    let assembly_name = assembly
        .assembly()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| package_name.to_string());
    let virtual_path = format!("ext:dotnet:{}/{}", package_name, assembly_name);
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();

    for type_def in assembly.types().all_types().iter() {
        let name = type_def.name.clone();
        let namespace = type_def.namespace.clone();
        if name.starts_with('<') || name == "<Module>" { continue }
        let visibility_mask = type_def.flags & 0x07;
        if visibility_mask != 1 && visibility_mask != 2 { continue }
        let is_interface = type_def.flags & 0x20 != 0;
        let kind = if is_interface { SymbolKind::Interface } else { SymbolKind::Class };

        let display_name = strip_backtick_arity(&name);
        let qualified_name = if namespace.is_empty() {
            display_name.to_string()
        } else {
            format!("{namespace}.{display_name}")
        };

        let type_generic_names: Vec<String> = type_def
            .generic_params
            .iter()
            .map(|(_, gp)| gp.name.clone())
            .collect();
        let type_gp_suffix = format_generic_suffix(&type_generic_names);

        symbols.push(ExtractedSymbol {
            name: display_name.to_string(),
            qualified_name: qualified_name.clone(),
            kind,
            visibility: Some(crate::types::Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: Some(format!(
                "{} {}{}",
                if is_interface { "interface" } else { "class" },
                display_name,
                type_gp_suffix
            )),
            doc_comment: None,
            scope_path: if namespace.is_empty() { None } else { Some(namespace.clone()) },
            parent_index: None,
        });

        for (_, method_ref) in type_def.methods.iter() {
            let Some(method) = method_ref.upgrade() else { continue };
            if method.name.starts_with('<') || method.name.starts_with('.') { continue }
            if method.flags_access != MethodAccessFlags::PUBLIC { continue }

            let method_name = method.name.clone();
            let method_qname = format!("{qualified_name}.{method_name}");
            let method_generic_names: Vec<String> = method
                .generic_params
                .iter()
                .map(|(_, gp)| gp.name.clone())
                .collect();
            let signature = format_method_signature(
                &method_name,
                &method.signature,
                &type_generic_names,
                &method_generic_names,
                &assembly,
            );
            symbols.push(ExtractedSymbol {
                name: method_name,
                qualified_name: method_qname,
                kind: SymbolKind::Method,
                visibility: Some(crate::types::Visibility::Public),
                start_line: 0, end_line: 0, start_col: 0, end_col: 0,
                signature: Some(signature),
                doc_comment: None,
                scope_path: Some(qualified_name.clone()),
                parent_index: None,
            });
        }
    }

    debug!("Parsed {} .NET symbols from {}", symbols.len(), dll_path.display());

    let metadata = std::fs::metadata(dll_path).map_err(|e| e.to_string())?;
    let size = metadata.len();
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    let content_hash = format!("{:x}", size).to_string();

    Ok(ParsedFile {
        path: virtual_path,
        language: lang_id.to_string(),
        content_hash,
        size,
        line_count: 0,
        mtime,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    })
}

fn strip_backtick_arity(name: &str) -> &str {
    match name.find('`') { Some(idx) => &name[..idx], None => name }
}

fn format_generic_suffix(names: &[String]) -> String {
    if names.is_empty() { String::new() } else { format!("<{}>", names.join(", ")) }
}

fn format_method_signature(
    method_name: &str,
    sig: &dotscope::metadata::signatures::SignatureMethod,
    type_generic_names: &[String],
    method_generic_names: &[String],
    assembly: &dotscope::prelude::CilObject,
) -> String {
    let gp_suffix = format_generic_suffix(method_generic_names);
    let mut params_str = String::from("(");
    for (i, p) in sig.params.iter().enumerate() {
        if i > 0 { params_str.push_str(", "); }
        let rendered = format!("{}", p);
        let substituted = substitute_generic_placeholders(&rendered, type_generic_names, method_generic_names);
        params_str.push_str(&resolve_signature_tokens(&substituted, assembly));
    }
    params_str.push(')');
    let return_rendered = format!("{}", sig.return_type);
    let return_substituted = substitute_generic_placeholders(&return_rendered, type_generic_names, method_generic_names);
    let return_str = resolve_signature_tokens(&return_substituted, assembly);
    format!("{method_name}{gp_suffix}{params_str}: {return_str}")
}

fn resolve_signature_tokens(
    rendered: &str,
    assembly: &dotscope::prelude::CilObject,
) -> String {
    use dotscope::metadata::token::Token;
    let type_registry = assembly.types();
    let imports = assembly.imports().cil();

    let mut out = String::with_capacity(rendered.len());
    let bytes = rendered.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let remaining = &rendered[i..];
        let (prefix_len, skip_prefix) = if remaining.starts_with("class[") {
            (6, true)
        } else if remaining.starts_with("valuetype[") {
            (10, true)
        } else {
            (0, false)
        };
        if skip_prefix {
            let after_prefix = &remaining[prefix_len..];
            if let Some(close_rel) = after_prefix.find(']') {
                let hex = &after_prefix[..close_rel];
                if let Ok(value) = u32::from_str_radix(hex, 16) {
                    let token = Token::new(value);
                    let table_byte = value >> 24;
                    let resolved: Option<String> = match table_byte {
                        0x02 => type_registry.get(&token).map(|ty| {
                            let name = strip_backtick_arity(&ty.name).to_string();
                            if ty.namespace.is_empty() { name } else { format!("{}.{}", ty.namespace, name) }
                        }),
                        0x01 => imports.get(token).map(|imp| {
                            let name = strip_backtick_arity(&imp.name).to_string();
                            if imp.namespace.is_empty() { name } else { format!("{}.{}", imp.namespace, name) }
                        }),
                        _ => None,
                    };
                    if let Some(full) = resolved {
                        out.push_str(&full);
                        i += prefix_len + close_rel + 1;
                        continue;
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn substitute_generic_placeholders(
    rendered: &str,
    type_gen: &[String],
    method_gen: &[String],
) -> String {
    let bytes = rendered.as_bytes();
    let mut out = String::with_capacity(rendered.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'!' {
            let is_method = i + 1 < bytes.len() && bytes[i + 1] == b'!';
            let num_start = if is_method { i + 2 } else { i + 1 };
            let mut num_end = num_start;
            while num_end < bytes.len() && bytes[num_end].is_ascii_digit() { num_end += 1 }
            if num_end > num_start {
                let idx: usize = rendered[num_start..num_end].parse().unwrap_or(usize::MAX);
                let target = if is_method { method_gen } else { type_gen };
                if let Some(name) = target.get(idx) {
                    out.push_str(name);
                    i = num_end;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn collect_dotnet_project_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 10 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(
                        name,
                        "bin" | "obj" | "node_modules" | ".git" | "target"
                            | "packages" | ".vs" | "TestResults" | "artifacts"
                    ) { continue }
                }
                collect_dotnet_project_files(&path, out, depth + 1);
            } else if ft.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if matches!(ext, "csproj" | "fsproj" | "vbproj") { out.push(path) }
                }
            }
        }
    }
}

// ===========================================================================
// NuGet source symbol index + resolve helpers (Ecosystem trait surface)
// ===========================================================================

/// Build a `SymbolLocationIndex` from `.cs` source files in the given dep
/// roots. Called via the `Ecosystem::build_symbol_index` method when the
/// pipeline constructs synthetic NuGet dep roots. In the primary eager
/// indexing flow the supplementary source scan happens inside
/// `parse_dotnet_externals_with_source` instead.
fn build_nuget_source_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut index = SymbolLocationIndex::new();
    for dep in dep_roots {
        for src_path in discover_nuget_source_files(&dep.root) {
            let Ok(content) = std::fs::read_to_string(&src_path) else { continue };
            for sym in scan_cs_header(&content) {
                index.insert(dep.module_path.clone(), sym.name, src_path.clone());
            }
        }
    }
    index
}

/// Return `WalkedFile` entries for `.cs` source files in `dep.root` that
/// declare any of the requested symbol short names.
fn resolve_nuget_source_symbols(dep: &ExternalDepRoot, symbols: &[&str]) -> Vec<WalkedFile> {
    if symbols.is_empty() { return Vec::new(); }
    let source_files = discover_nuget_source_files(&dep.root);
    if source_files.is_empty() { return Vec::new(); }

    let targets: std::collections::HashSet<&str> = symbols.iter().copied().collect();
    let mut out = Vec::new();

    for src_path in source_files {
        let Ok(content) = std::fs::read_to_string(&src_path) else { continue };
        let decls = scan_cs_header(&content);
        if decls.iter().any(|d| targets.contains(d.name.as_str())) {
            let rel = src_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push(WalkedFile {
                relative_path: format!("ext:dotnet-src:{}/{}", dep.module_path, rel),
                absolute_path: src_path,
                language: "csharp",
            });
        }
    }
    out
}

// ===========================================================================
// NuGet source discovery + header-only C# parsing
// ===========================================================================

/// Discover `.cs` source files shipped inside a NuGet package version dir.
/// Checks in priority order:
///   1. `contentFiles/cs/<tfm>/**/*.cs` — NuGet contentFiles convention
///   2. `lib/<tfm>/**/*.cs` — rare but exists in some packages
///   3. `src/**/*.cs` — source-only packages (Microsoft.Bcl.*, etc.)
///   4. Top-level `*.cs` at the package root
///
/// Returns deduped absolute paths.
fn discover_nuget_source_files(version_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    // 1. contentFiles/cs/<tfm>/
    let content_files_cs = version_dir.join("contentFiles").join("cs");
    if content_files_cs.is_dir() {
        let preferred_tfms = [
            "net9.0", "net8.0", "net7.0", "net6.0",
            "netstandard2.1", "netstandard2.0", "any",
        ];
        let tfm_dir = preferred_tfms.iter()
            .map(|tfm| content_files_cs.join(tfm))
            .find(|p| p.is_dir())
            .or_else(|| largest_subdir(&content_files_cs));
        if let Some(dir) = tfm_dir {
            collect_cs_files(&dir, &mut out, &mut seen, 0);
        }
    }

    // 2. lib/<tfm>/**/*.cs
    let lib_dir = version_dir.join("lib");
    if lib_dir.is_dir() {
        let preferred_tfms = [
            "net9.0", "net8.0", "net7.0", "net6.0",
            "netstandard2.1", "netstandard2.0",
        ];
        let tfm_dir = preferred_tfms.iter()
            .map(|tfm| lib_dir.join(tfm))
            .find(|p| p.is_dir())
            .or_else(|| largest_subdir(&lib_dir));
        if let Some(dir) = tfm_dir {
            collect_cs_files(&dir, &mut out, &mut seen, 0);
        }
    }

    // 3. src/
    let src_dir = version_dir.join("src");
    if src_dir.is_dir() {
        collect_cs_files(&src_dir, &mut out, &mut seen, 0);
    }

    // 4. Top-level *.cs
    if let Ok(entries) = std::fs::read_dir(version_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "cs") {
                if seen.insert(path.clone()) { out.push(path); }
            }
        }
    }

    out
}

/// Recursive `.cs` collector with depth cap. Skips build-artifact and
/// test subdirectories.
fn collect_cs_files(
    dir: &Path,
    out: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: usize,
) {
    if depth > 8 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "obj" | "bin" | "test" | "tests" | "samples" | "examples" | ".git"
                ) { continue; }
            }
            collect_cs_files(&path, out, seen, depth + 1);
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "cs") {
            if seen.insert(path.clone()) { out.push(path); }
        }
    }
}

/// Parse a single `.cs` source file header-only, returning a synthetic
/// `ParsedFile`. Uses `ext:dotnet-src:<pkg_name>/<filename>` as the virtual
/// path so it's distinguishable from DLL-synthesized rows. Real line numbers
/// are preserved for chain walkers.
fn parse_cs_source_file(
    path: &Path,
    pkg_name: &str,
    lang_id: &str,
) -> std::result::Result<crate::types::ParsedFile, String> {
    use crate::types::{ExtractedSymbol, ParsedFile, Visibility};

    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let decls = scan_cs_header(&content);

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown.cs".to_string());
    let virtual_path = format!("ext:dotnet-src:{pkg_name}/{file_name}");

    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let size = metadata.len();
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    let content_hash = format!("{:x}", size);
    let line_count = content.lines().count() as u32;

    let extracted: Vec<ExtractedSymbol> = decls
        .into_iter()
        .map(|sym| ExtractedSymbol {
            name: sym.name.clone(),
            qualified_name: if sym.scope.is_empty() {
                sym.name.clone()
            } else {
                format!("{}.{}", sym.scope, sym.name)
            },
            kind: sym.kind,
            visibility: Some(Visibility::Public),
            start_line: sym.line as u32,
            end_line: sym.line as u32,
            start_col: 0,
            end_col: 0,
            signature: sym.signature,
            doc_comment: None,
            scope_path: if sym.scope.is_empty() { None } else { Some(sym.scope) },
            parent_index: None,
        })
        .collect();

    Ok(ParsedFile {
        path: virtual_path,
        language: lang_id.to_string(),
        content_hash,
        size,
        line_count,
        mtime,
        package_id: None,
        symbols: extracted,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    })
}

// ===========================================================================
// C# header-only scanner
// ===========================================================================
//
// Line-based extraction of top-level declarations from `.cs` source files.
// Tracks brace depth to determine scope (namespace, type nesting). Emits
// public/internal type declarations and public methods — skips private
// members, compiler-generated names, and method body interiors.

#[derive(Debug)]
struct CsDecl {
    name: String,
    /// Dot-joined namespace + enclosing type path (empty at global scope).
    scope: String,
    kind: crate::types::SymbolKind,
    signature: Option<String>,
    /// 1-based source line number.
    line: usize,
}

/// Scan a C# source file and extract top-level public declarations.
/// Returns one `CsDecl` per class/interface/enum/struct/record/delegate
/// and public method found at namespace→type→member depth.
pub(crate) fn scan_cs_header(source: &str) -> Vec<CsDecl> {
    use crate::types::SymbolKind;

    let mut out = Vec::new();
    // Stack entries: (name, kind_char) where 'n'=namespace, 't'=type.
    let mut scope_stack: Vec<(String, char)> = Vec::new();

    for (line_idx, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim();

        // Count brace deltas on this line.
        let opens = line.chars().filter(|&c| c == '{').count() as i32;
        let closes = line.chars().filter(|&c| c == '}').count() as i32;

        // Pop scope for net closing braces (handles standalone `}` lines).
        if closes > opens {
            let net_close = (closes - opens) as usize;
            for _ in 0..net_close.min(scope_stack.len()) {
                scope_stack.pop();
            }
        }

        // Skip non-declaration lines early.
        if line.is_empty()
            || line.starts_with("//")
            || line.starts_with("/*")
            || line.starts_with('*')
            || line.starts_with('[')
            || line.starts_with('#')
        {
            continue;
        }

        // Namespace declaration — capture the full dotted name
        // (`namespace Acme.Orders` → "Acme.Orders").
        if let Some(rest) = strip_cs_keyword(line, "namespace") {
            let ns_name = cs_namespace_name(rest);
            if !ns_name.is_empty() {
                scope_stack.push((ns_name, 'n'));
            }
            continue;
        }

        // Skip non-public members (private/protected/internal-only at type level).
        let is_public = line.contains("public ")
            || (!line.contains("private ")
                && !line.contains("protected ")
                && !line.contains("internal "));
        if !is_public { continue; }

        // Type declarations.
        let type_kw: Option<(&str, SymbolKind)> = [
            ("interface ", SymbolKind::Interface),
            ("class ",     SymbolKind::Class),
            ("struct ",    SymbolKind::Struct),
            ("enum ",      SymbolKind::Enum),
            ("record ",    SymbolKind::Class),
            ("delegate ",  SymbolKind::Function),
        ]
        .iter()
        .find_map(|(kw, kind)| {
            if line.contains(kw) { Some((*kw, *kind)) } else { None }
        });

        if let Some((kw, kind)) = type_kw {
            if let Some(pos) = line.find(kw) {
                let after_kw = &line[pos + kw.len()..];
                let type_name = cs_first_ident(after_kw);
                if !type_name.is_empty() && !type_name.starts_with('<') {
                    let scope = cs_scope_string(&scope_stack, 'n');
                    let sig = Some(format!("{}{}", kw.trim_end(), format!(" {type_name}")));
                    out.push(CsDecl {
                        name: type_name.clone(),
                        scope,
                        kind,
                        signature: sig,
                        line: line_idx + 1,
                    });
                    scope_stack.push((type_name, 't'));
                }
            }
            continue;
        }

        // Method declarations — only emit if directly inside a type scope.
        let inside_type = scope_stack.last().map(|(_, k)| *k == 't').unwrap_or(false);
        if !inside_type { continue; }
        if line.contains("operator ") { continue; }

        if line.contains('(') {
            let method_name = extract_cs_method_name(line);
            if !method_name.is_empty() && !is_cs_noise_ident(&method_name) {
                let scope = cs_scope_string(&scope_stack, 't');
                let sig = Some(truncate_to_paren(line, 120));
                out.push(CsDecl {
                    name: method_name,
                    scope,
                    kind: SymbolKind::Method,
                    signature: sig,
                    line: line_idx + 1,
                });
            }
        }
    }

    out
}

/// Dot-join scope names of the requested kind and above.
/// `min_kind='n'` collects only namespace segments.
/// `min_kind='t'` collects namespace + type segments.
fn cs_scope_string(stack: &[(String, char)], min_kind: char) -> String {
    stack
        .iter()
        .filter(|(_, k)| *k == 'n' || (min_kind == 't' && *k == 't'))
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

/// Return the suffix after `kw ` in `line` if the keyword is present.
fn strip_cs_keyword<'a>(line: &'a str, kw: &str) -> Option<&'a str> {
    let pattern = format!("{kw} ");
    line.find(&pattern).map(|pos| &line[pos + pattern.len()..])
}

/// Grab the first C# identifier from `s` (alphanumeric + underscore).
fn cs_first_ident(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

/// Grab a dotted namespace name from `s` (alphanumeric + `_` + `.`).
/// Stops at whitespace, `{`, or `;`. Used for `namespace Acme.Orders`.
fn cs_namespace_name(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
        .collect::<String>()
        .trim_end_matches('.')
        .to_string()
}

/// Extract the method name from a line like
/// `public async Task<T> MyMethod(...)` — last identifier before `(`.
fn extract_cs_method_name(line: &str) -> String {
    let paren = match line.find('(') { Some(p) => p, None => return String::new() };
    let before_paren = line[..paren].trim_end();
    // Strip trailing generic suffix `<T>` before the paren.
    let before_paren = if before_paren.ends_with('>') {
        match before_paren.rfind('<') {
            Some(lt) => before_paren[..lt].trim_end(),
            None => before_paren,
        }
    } else {
        before_paren
    };
    // Walk backwards to extract trailing identifier.
    before_paren
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

/// Truncate a line to `max_len` chars but keep up to the first `)`.
fn truncate_to_paren(line: &str, max_len: usize) -> String {
    let end = line.find(')').map(|p| (p + 1).min(line.len())).unwrap_or(line.len());
    let s = &line[..end.min(line.len())];
    if s.len() > max_len { s[..max_len].to_string() } else { s.to_string() }
}

/// True for C# keywords and noise identifiers that can never be method names.
fn is_cs_noise_ident(name: &str) -> bool {
    matches!(
        name,
        "if" | "else" | "for" | "foreach" | "while" | "do" | "switch"
            | "catch" | "finally" | "using" | "return" | "new" | "throw"
            | "var" | "get" | "set" | "init" | "add" | "remove"
    ) || name.starts_with('<')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let n = NugetEcosystem;
        assert_eq!(n.id(), ID);
        assert_eq!(Ecosystem::kind(&n), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&n), &["csharp", "fsharp", "vbnet"]);
    }

    #[test]
    fn legacy_locator_tag_is_dotnet() {
        assert_eq!(ExternalSourceLocator::ecosystem(&NugetEcosystem), "dotnet");
    }

    #[test]
    fn strip_backtick_arity_removes_generic_suffix() {
        assert_eq!(strip_backtick_arity("Repository`1"), "Repository");
        assert_eq!(strip_backtick_arity("Dictionary`2"), "Dictionary");
        assert_eq!(strip_backtick_arity("Func`4"), "Func");
        assert_eq!(strip_backtick_arity("List"), "List");
    }

    #[test]
    fn format_generic_suffix_joins_names() {
        assert_eq!(format_generic_suffix(&[]), "");
        assert_eq!(format_generic_suffix(&["T".to_string()]), "<T>");
        assert_eq!(
            format_generic_suffix(&["T".to_string(), "U".to_string()]),
            "<T, U>"
        );
    }

    #[test]
    fn substitute_placeholders_swaps_ecma335_syntax() {
        let type_gen = vec!["T".to_string()];
        let method_gen = vec!["U".to_string(), "V".to_string()];
        assert_eq!(substitute_generic_placeholders("!!0", &type_gen, &method_gen), "U");
        assert_eq!(substitute_generic_placeholders("!!1", &type_gen, &method_gen), "V");
        assert_eq!(substitute_generic_placeholders("!0", &type_gen, &method_gen), "T");
        assert_eq!(
            substitute_generic_placeholders("Func<!0, !!0, !!1>", &type_gen, &method_gen),
            "Func<T, U, V>"
        );
        assert_eq!(substitute_generic_placeholders("!!5", &type_gen, &method_gen), "!!5");
    }

    #[test]
    fn substitute_placeholders_multi_digit_indices() {
        let method_gen: Vec<String> = (0..15).map(|i| format!("T{i}")).collect();
        assert_eq!(substitute_generic_placeholders("!!10", &[], &method_gen), "T10");
        assert_eq!(substitute_generic_placeholders("!!14", &[], &method_gen), "T14");
    }

    #[test]
    fn project_references_extract_filename_stems() {
        let csproj = r#"
            <Project Sdk="Microsoft.NET.Sdk">
              <ItemGroup>
                <ProjectReference Include="../Shared/Shared.csproj" />
                <ProjectReference Include="..\Infra\Infra.fsproj" />
                <ProjectReference Include="./Legacy.vbproj" />
              </ItemGroup>
            </Project>
        "#;
        let refs = parse_project_references(csproj);
        assert_eq!(refs, vec!["Shared", "Infra", "Legacy"]);
    }

    #[test]
    fn project_references_kept_separate_from_packages() {
        let csproj = r#"
            <Project Sdk="Microsoft.NET.Sdk">
              <ItemGroup>
                <PackageReference Include="Newtonsoft.Json" Version="13.0.1" />
                <ProjectReference Include="../Shared/Shared.csproj" />
              </ItemGroup>
            </Project>
        "#;
        let pkgs = parse_package_references(csproj);
        let prs = parse_project_references(csproj);
        assert_eq!(pkgs, vec!["Newtonsoft.Json"]);
        assert_eq!(prs, vec!["Shared"]);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }

    // -----------------------------------------------------------------------
    // C# header scanner tests
    // -----------------------------------------------------------------------

    #[test]
    fn scan_cs_header_finds_class_and_interface() {
        let src = r#"
namespace MyLib.Core {
    public interface IRepository {
        IEnumerable<T> GetAll();
    }
    public class UserRepository : IRepository {
        public IEnumerable<T> GetAll() { return null; }
    }
}
"#;
        let decls = scan_cs_header(src);
        let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"IRepository"), "should find IRepository");
        assert!(names.contains(&"UserRepository"), "should find UserRepository");
    }

    #[test]
    fn scan_cs_header_captures_namespace_scope() {
        let src = r#"
namespace Acme.Orders {
    public class OrderService { }
}
"#;
        let decls = scan_cs_header(src);
        let svc = decls.iter().find(|d| d.name == "OrderService").expect("OrderService missing");
        assert_eq!(svc.scope, "Acme.Orders");
    }

    #[test]
    fn scan_cs_header_skips_private_members() {
        let src = r#"
namespace X {
    public class Foo {
        private void Secret() { }
        public void Public() { }
    }
}
"#;
        let decls = scan_cs_header(src);
        let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
        assert!(!names.contains(&"Secret"), "should not emit private method");
        assert!(names.contains(&"Public"), "should emit public method");
    }

    #[test]
    fn scan_cs_header_handles_enum_and_struct() {
        let src = r#"
namespace Lib {
    public enum Status { Active, Inactive }
    public struct Point { public int X; public int Y; }
}
"#;
        let decls = scan_cs_header(src);
        let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"Status"));
        assert!(names.contains(&"Point"));
    }

    #[test]
    fn discover_nuget_source_files_empty_for_missing_dir() {
        let tmp = std::env::temp_dir().join("bw-nuget-test-nonexistent-xyz");
        let files = discover_nuget_source_files(&tmp);
        assert!(files.is_empty());
    }

    #[test]
    fn discover_nuget_source_finds_contentfiles() {
        let tmp = std::env::temp_dir().join("bw-nuget-test-contentfiles");
        let cs_dir = tmp.join("contentFiles").join("cs").join("any");
        std::fs::create_dir_all(&cs_dir).unwrap();
        std::fs::write(cs_dir.join("Helper.cs"), "public class Helper {}").unwrap();
        let files = discover_nuget_source_files(&tmp);
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("Helper.cs"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn discover_nuget_source_finds_src_dir() {
        let tmp = std::env::temp_dir().join("bw-nuget-test-src");
        let src_dir = tmp.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("MyClass.cs"), "public class MyClass {}").unwrap();
        let files = discover_nuget_source_files(&tmp);
        assert_eq!(files.len(), 1);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
