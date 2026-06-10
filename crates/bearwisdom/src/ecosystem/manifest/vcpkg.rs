// =============================================================================
// ecosystem/manifest/vcpkg.rs — `vcpkg.json` manifest reader
//
// vcpkg's manifest mode declares C/C++ deps in a per-project `vcpkg.json`:
//
//   {
//     "name": "myapp",
//     "dependencies": ["fmt", "boost-asio", { "name": "qt", "features": [...] }]
//   }
//
// Each entry is either a string (the port name) or an object with a
// `name` field. Resolvers consume the port-name set to classify bare
// `#include <fmt/...>` refs as external rather than unresolved.
// =============================================================================

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

pub struct VcpkgManifest;

impl ManifestReader for VcpkgManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Vcpkg
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let path = project_root.join("vcpkg.json");
        let content = std::fs::read_to_string(&path).ok()?;
        let data = parse_vcpkg_json(&content);
        Some(data)
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let mut out = Vec::new();
        collect_vcpkg_manifests(project_root, &mut out, 0);
        out
    }
}

fn collect_vcpkg_manifests(dir: &Path, out: &mut Vec<ReaderEntry>, depth: u32) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                name,
                "node_modules" | ".git" | "target" | "build" | ".bearwisdom" | "vcpkg_installed"
            ) {
                continue;
            }
            collect_vcpkg_manifests(&path, out, depth + 1);
        } else if path.file_name().and_then(|n| n.to_str()) == Some("vcpkg.json") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let data = parse_vcpkg_json(&content);
                let name = parse_vcpkg_name(&content);
                out.push(ReaderEntry {
                    package_dir: path.parent().unwrap_or(dir).to_path_buf(),
                    manifest_path: path,
                    data,
                    name,
                });
            }
        }
    }
}

/// Minimal JSON parser for the `dependencies` field. Uses
/// `serde_json` (already a workspace dep). Falls back to empty on
/// malformed input.
pub fn parse_vcpkg_json(content: &str) -> ManifestData {
    let mut data = ManifestData::default();
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
        return data;
    };
    if let Some(deps) = val.get("dependencies").and_then(|v| v.as_array()) {
        for dep in deps {
            let name = match dep {
                serde_json::Value::String(s) => Some(s.as_str()),
                serde_json::Value::Object(o) => o.get("name").and_then(|v| v.as_str()),
                _ => None,
            };
            if let Some(n) = name {
                data.dependencies.insert(n.to_string());
            }
        }
    }
    data
}

pub fn parse_vcpkg_name(content: &str) -> Option<String> {
    let val = serde_json::from_str::<serde_json::Value>(content).ok()?;
    val.get("name")?.as_str().map(|s| s.to_string())
}

#[cfg(test)]
#[path = "vcpkg_tests.rs"]
mod tests;
