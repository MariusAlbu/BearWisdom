// =============================================================================
// nuget/manifest.rs — NuGetManifest reader + .csproj parsing helpers.
//
// Reads `*.csproj` / `*.fsproj` / `*.vbproj` (and `Directory.Packages.props`
// via the same parsers) to surface PackageReference / ProjectReference items
// + SDK type + implicit usings + global usings declared anywhere in the tree.
// =============================================================================

use std::path::{Path, PathBuf};

use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

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

pub(crate) fn find_csproj_files(root: &Path) -> Vec<PathBuf> {
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

/// Slice the leading bytes of `s` up to `max` bytes, walking back to a
/// UTF-8 char boundary. Avoids panics on `.csproj` payloads with
/// non-ASCII metadata (Chinese/Japanese package descriptions, etc.).
fn clamp_to_char_boundary(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub fn parse_project_references(content: &str) -> Vec<String> {
    let tag = "ProjectReference";
    let mut out = Vec::new();
    let mut search_from = 0;
    while let Some(pos) = content[search_from..].find(tag) {
        let abs_pos = search_from + pos;
        search_from = abs_pos + tag.len();
        let rest = &content[search_from..];
        let window = clamp_to_char_boundary(rest, 512);
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
        let window = clamp_to_char_boundary(rest, 256);
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
