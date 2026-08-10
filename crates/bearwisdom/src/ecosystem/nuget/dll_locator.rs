// =============================================================================
// nuget/dll_locator.rs — locate the DLLs a .NET project declares.
//
// Walks the project's `*.csproj` / `.fsproj` / `.vbproj` files, collects
// PackageReference coords plus the transitive closure from `*.deps.json`
// (build artifact) and `obj/project.assets.json` (restore artifact), and maps
// each coord to its preferred-TFM DLL under the NuGet cache. Discovery only —
// no parsing, no symbols.
// =============================================================================

use std::path::{Path, PathBuf};
use super::version_select::select_version_subdir;

use super::manifest::{parse_package_references_full, NuGetCoord};

/// Locate every DLL the project declares, without emitting any parsed symbols.
/// Returns `(package_name, dll_abs_path, lang_id)` for each discovered DLL.
/// Used by the demand-driven path to build a `SymbolLocationIndex` cheaply.
pub(crate) fn locate_dlls_for_project(
    project_root: &Path,
) -> Vec<(String, PathBuf, &'static str)> {
    let mut project_files: Vec<PathBuf> = Vec::new();
    collect_dotnet_project_files(project_root, &mut project_files, 0);
    if project_files.is_empty() {
        return Vec::new();
    }
    let lang_id = dominant_dotnet_language(&project_files);
    let mut coords: Vec<NuGetCoord> = Vec::new();
    for p in &project_files {
        let Ok(content) = std::fs::read_to_string(p) else {
            continue;
        };
        coords.extend(parse_package_references_full(&content));
    }
    for p in &project_files {
        if let Some(proj_dir) = p.parent() {
            coords.extend(collect_transitive_coords_from_deps_json(proj_dir));
            coords.extend(collect_transitive_coords_from_assets_json(proj_dir));
        }
    }
    if coords.is_empty() {
        return Vec::new();
    }
    let Some(nuget_root) = nuget_packages_root() else {
        return Vec::new();
    };
    // Deduplicate by DLL path to avoid re-indexing the same assembly twice.
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for coord in &coords {
        let pkg_dir = nuget_root.join(coord.name.to_lowercase());
        if !pkg_dir.is_dir() {
            continue;
        }
        let version = match select_version_subdir(&pkg_dir, coord.version.as_deref()) {
            Some(v) => v,
            None => continue,
        };
        let version_dir = pkg_dir.join(&version);
        for dll_path in find_dlls_in_version_dir(&version_dir, &coord.name) {
            if seen.insert(dll_path.clone()) {
                out.push((coord.name.clone(), dll_path, lang_id));
            }
        }
    }
    out
}

pub(super) fn collect_transitive_coords_from_deps_json(proj_dir: &Path) -> Vec<NuGetCoord> {
    let mut deps_json_files: Vec<PathBuf> = Vec::new();
    collect_deps_json(&proj_dir.join("bin"), &mut deps_json_files, 0);
    if deps_json_files.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in deps_json_files.iter().take(16) {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        let Some(libs) = json.get("libraries").and_then(|v| v.as_object()) else {
            continue;
        };
        for (key, value) in libs {
            let ty = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if ty != "package" {
                continue;
            }
            let Some((name, version)) = key.rsplit_once('/') else {
                continue;
            };
            if !seen.insert(key.clone()) {
                continue;
            }
            out.push(NuGetCoord {
                name: name.to_string(),
                version: Some(version.to_string()),
            });
        }
    }
    out
}

/// Transitive package coordinates from the RESTORE artifact
/// `obj/project.assets.json` — present after `dotnet restore` with no build.
/// Same `libraries` shape as `*.deps.json` (`"Pkg/1.2.3": {"type":"package"}`),
/// so a restored-but-never-built project still surfaces its full transitive
/// closure (the packages that declare `IdentityUser`, `ModelBuilder`, …).
pub(super) fn collect_transitive_coords_from_assets_json(proj_dir: &Path) -> Vec<NuGetCoord> {
    let path = proj_dir.join("obj").join("project.assets.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Vec::new();
    };
    let Some(libs) = json.get("libraries").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, value) in libs {
        if value.get("type").and_then(|t| t.as_str()) != Some("package") {
            continue;
        }
        let Some((name, version)) = key.rsplit_once('/') else {
            continue;
        };
        out.push(NuGetCoord {
            name: name.to_string(),
            version: Some(version.to_string()),
        });
    }
    out
}

fn collect_deps_json(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 5 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(name, "obj" | "runtimes" | "ref") {
                        continue;
                    }
                }
                collect_deps_json(&path, out, depth + 1);
            } else if ft.is_file()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".deps.json"))
            {
                out.push(path);
            }
        }
    }
}

pub(super) fn dominant_dotnet_language(project_files: &[PathBuf]) -> &'static str {
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
    if cs >= fs && cs >= vb {
        "csharp"
    } else if fs >= vb {
        "fsharp"
    } else {
        "vbnet"
    }
}

pub fn nuget_packages_root() -> Option<PathBuf> {
    for key in ["BEARWISDOM_NUGET_PACKAGES", "NUGET_PACKAGES"] {
        if let Some(raw) = std::env::var_os(key) {
            let p = PathBuf::from(raw);
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join(".nuget").join("packages");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

/// Locate every managed `.dll` in the preferred-TFM `lib/` directory of an
/// already-resolved `<nuget-cache>/<pkg-id>/<version>/` directory. All DLLs in
/// the chosen TFM dir are the package's compile assets — a package id and its
/// assembly name are independent, so matching only `<pkg-id>.dll` silently
/// drops packages whose assembly is named differently. The `<pkg-id>.dll`
/// exact match sorts first as the primary assembly. Empty for source-only
/// packages that ship no `lib/` directory.
pub(super) fn find_dlls_in_version_dir(version_dir: &Path, pkg_name: &str) -> Vec<PathBuf> {
    let lib_dir = version_dir.join("lib");
    if !lib_dir.is_dir() {
        return Vec::new();
    }

    let preferred_tfms = [
        "net9.0",
        "net8.0",
        "net7.0",
        "net6.0",
        "netstandard2.1",
        "netstandard2.0",
    ];
    let mut chosen_tfm: Option<PathBuf> = None;
    for tfm in preferred_tfms {
        let candidate = lib_dir.join(tfm);
        if candidate.is_dir() {
            chosen_tfm = Some(candidate);
            break;
        }
    }
    let Some(tfm_dir) = chosen_tfm.or_else(|| largest_subdir(&lib_dir)) else {
        return Vec::new();
    };

    let Ok(entries) = std::fs::read_dir(&tfm_dir) else {
        return Vec::new();
    };
    let target_lower = pkg_name.to_lowercase() + ".dll";
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .to_lowercase()
                .ends_with(".dll")
        })
        .map(|e| e.path())
        .collect();
    out.sort_by_key(|p| {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        (name != target_lower, name)
    });
    out
}


pub(crate) fn largest_subdir(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subs: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            if e.file_type().ok()?.is_dir() {
                Some(e.path())
            } else {
                None
            }
        })
        .collect();
    subs.sort();
    subs.into_iter().next_back()
}

pub(super) fn collect_dotnet_project_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 10 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(
                        name,
                        "bin"
                            | "obj"
                            | "node_modules"
                            | ".git"
                            | "target"
                            | "packages"
                            | ".vs"
                            | "TestResults"
                            | "artifacts"
                    ) {
                        continue;
                    }
                }
                collect_dotnet_project_files(&path, out, depth + 1);
            } else if ft.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if matches!(ext, "csproj" | "fsproj" | "vbproj") {
                        out.push(path)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "dll_locator_tests.rs"]
mod tests;
