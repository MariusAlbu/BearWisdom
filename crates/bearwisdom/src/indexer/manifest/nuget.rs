// indexer/manifest/nuget.rs — .csproj / NuGet reader

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct NuGetManifest;

impl ManifestReader for NuGetManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::NuGet
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let csproj_files = find_csproj_files(project_root);
        if csproj_files.is_empty() {
            return None;
        }

        let mut data = ManifestData::default();
        let mut sdk_types = Vec::new();

        for csproj_path in &csproj_files {
            let content = match std::fs::read_to_string(csproj_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if let Some(sdk) = parse_sdk_type(&content) {
                sdk_types.push(sdk);
            }

            for pkg in parse_package_references(&content) {
                data.dependencies.insert(pkg);
            }
        }

        // Determine the most capable SDK type across all project files.
        let sdk = most_capable_sdk(&sdk_types);
        data.sdk_type = Some(sdk_type_name(sdk).to_string());

        // Collect SDK implicit usings.
        let implicit = implicit_usings_for_sdk(sdk);
        for ns in implicit {
            if !data.global_usings.contains(&ns.to_string()) {
                data.global_usings.push(ns.to_string());
            }
        }

        // Scan for GlobalUsings.cs files.
        let global_using_files = find_global_using_files(project_root);
        for path in &global_using_files {
            if let Ok(content) = std::fs::read_to_string(path) {
                for ns in parse_global_usings(&content) {
                    if !data.global_usings.contains(&ns) {
                        data.global_usings.push(ns);
                    }
                }
            }
        }

        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn find_csproj_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut result = Vec::new();
    collect_csproj(root, &mut result, 0);
    result
}

fn collect_csproj(dir: &Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
    if depth > 10 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                "bin" | "obj" | "node_modules" | ".git" | "target"
                    | "packages" | ".vs" | "TestResults" | "artifacts"
            ) {
                continue;
            }
            collect_csproj(&path, out, depth + 1);
        } else if path.extension().is_some_and(|e| {
            e == "csproj" || e == "fsproj" || e == "vbproj"
        }) {
            out.push(path);
        }
    }
}

/// .NET SDK type, determines which implicit usings are injected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DotnetSdkType {
    /// Microsoft.NET.Sdk — console/library projects
    Base,
    /// Microsoft.NET.Sdk.Web — ASP.NET Core projects
    Web,
    /// Microsoft.NET.Sdk.Worker — background service projects
    Worker,
    /// Microsoft.NET.Sdk.BlazorWebAssembly
    Blazor,
    /// Unknown SDK string
    Other,
}

fn sdk_type_name(sdk: DotnetSdkType) -> &'static str {
    match sdk {
        DotnetSdkType::Base => "base",
        DotnetSdkType::Web => "web",
        DotnetSdkType::Worker => "worker",
        DotnetSdkType::Blazor => "blazor",
        DotnetSdkType::Other => "other",
    }
}

/// Extract the SDK type from a .csproj file's `<Project Sdk="...">` attribute.
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

/// Extract `<PackageReference Include="..." />` names from .csproj content.
pub fn parse_package_references(content: &str) -> Vec<String> {
    parse_package_references_full(content)
        .into_iter()
        .map(|c| c.name)
        .collect()
}

/// A NuGet package coordinate extracted from a .csproj `<PackageReference>`.
/// Needed by externals discovery to probe the NuGet global packages folder
/// at `~/.nuget/packages/{lowercased_name}/{version}/`.
#[derive(Debug, Clone)]
pub struct NuGetCoord {
    pub name: String,
    /// None when the csproj omits `Version=` or uses a variable
    /// (`Version="$(Foo)"`) we can't resolve here. Discovery falls back to
    /// a version-directory scan in that case.
    pub version: Option<String>,
}

/// Extract full `(name, version)` tuples from `<PackageReference ... />`
/// entries in a .csproj. Handles both self-closing (`<PackageReference
/// Include="..." Version="..." />`) and paired (`<PackageReference
/// Include="...">...</PackageReference>`) forms.
pub fn parse_package_references_full(content: &str) -> Vec<NuGetCoord> {
    let mut coords = Vec::new();
    let tag = "PackageReference";

    let mut search_from = 0;
    while let Some(pos) = content[search_from..].find(tag) {
        let abs_pos = search_from + pos;
        search_from = abs_pos + tag.len();

        let rest = &content[search_from..];
        // 256 bytes is enough for any well-formed tag including Version.
        let window = &rest[..rest.len().min(256)];
        let name = window.find("Include=\"").and_then(|inc_pos| {
            let after_inc = &window[inc_pos + 9..];
            after_inc
                .find('"')
                .map(|end| after_inc[..end].to_string())
                .filter(|s| !s.is_empty())
        });
        let Some(name) = name else { continue };

        let version = window.find("Version=\"").and_then(|ver_pos| {
            let after_ver = &window[ver_pos + 9..];
            after_ver
                .find('"')
                .map(|end| after_ver[..end].to_string())
                .filter(|v| !v.is_empty() && !v.starts_with("$("))
        });

        coords.push(NuGetCoord { name, version });
    }

    coords
}

/// Pick the "most capable" SDK from a list — Web > Worker > Blazor > Base.
pub fn most_capable_sdk(sdks: &[DotnetSdkType]) -> DotnetSdkType {
    if sdks.contains(&DotnetSdkType::Web) {
        DotnetSdkType::Web
    } else if sdks.contains(&DotnetSdkType::Worker) {
        DotnetSdkType::Worker
    } else if sdks.contains(&DotnetSdkType::Blazor) {
        DotnetSdkType::Blazor
    } else if sdks.contains(&DotnetSdkType::Base) {
        DotnetSdkType::Base
    } else {
        DotnetSdkType::Other
    }
}

/// Return the implicit usings for a given .NET SDK type.
pub fn implicit_usings_for_sdk(sdk: DotnetSdkType) -> Vec<&'static str> {
    let mut usings = vec![
        "System",
        "System.Collections.Generic",
        "System.IO",
        "System.Linq",
        "System.Net.Http",
        "System.Threading",
        "System.Threading.Tasks",
    ];

    match sdk {
        DotnetSdkType::Web => {
            usings.extend_from_slice(&[
                "System.Net.Http.Json",
                "Microsoft.AspNetCore.Builder",
                "Microsoft.AspNetCore.Hosting",
                "Microsoft.AspNetCore.Http",
                "Microsoft.AspNetCore.Http.HttpResults",
                "Microsoft.AspNetCore.Mvc",
                "Microsoft.AspNetCore.Routing",
                "Microsoft.Extensions.Configuration",
                "Microsoft.Extensions.DependencyInjection",
                "Microsoft.Extensions.Hosting",
                "Microsoft.Extensions.Logging",
            ]);
        }
        DotnetSdkType::Worker => {
            usings.extend_from_slice(&[
                "Microsoft.Extensions.Configuration",
                "Microsoft.Extensions.DependencyInjection",
                "Microsoft.Extensions.Hosting",
                "Microsoft.Extensions.Logging",
            ]);
        }
        DotnetSdkType::Blazor => {
            usings.extend_from_slice(&[
                "System.Net.Http.Json",
                "Microsoft.AspNetCore.Components",
                "Microsoft.AspNetCore.Components.Forms",
                "Microsoft.AspNetCore.Components.Routing",
                "Microsoft.AspNetCore.Components.Web",
                "Microsoft.Extensions.Configuration",
                "Microsoft.Extensions.DependencyInjection",
                "Microsoft.Extensions.Logging",
            ]);
        }
        _ => {}
    }

    usings
}

fn find_global_using_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut result = Vec::new();
    collect_global_usings(root, &mut result, 0);
    result
}

fn collect_global_usings(dir: &Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
    if depth > 10 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                "bin" | "obj" | "node_modules" | ".git" | "target"
                    | "packages" | ".vs" | "TestResults" | "artifacts"
            ) {
                continue;
            }
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

/// Parse `global using ...;` statements from a .cs file.
pub fn parse_global_usings(content: &str) -> Vec<String> {
    let mut usings = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("global using") {
            let rest = rest.trim();
            if rest.starts_with("static ") {
                continue;
            }
            let ns = rest.trim_end_matches(';').trim();
            if !ns.is_empty() {
                usings.push(ns.to_string());
            }
        }
    }
    usings
}
