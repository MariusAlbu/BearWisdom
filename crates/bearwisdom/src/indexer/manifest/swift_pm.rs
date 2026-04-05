// indexer/manifest/swift_pm.rs — Package.swift reader

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct SwiftPMManifest;

impl ManifestReader for SwiftPMManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::SwiftPM
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let package_swift = project_root.join("Package.swift");
        if !package_swift.is_file() {
            return None;
        }
        let content = std::fs::read_to_string(&package_swift).ok()?;

        let mut data = ManifestData::default();
        for name in parse_swift_package_deps(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse dependency names from a `Package.swift` manifest.
///
/// Handles both URL-based and name-based package entries:
///   `.package(url: "https://github.com/apple/swift-argument-parser", from: "1.0.0")`
///   `.package(name: "Vapor", url: "https://github.com/vapor/vapor.git", ...)`
///   `.package(url: "https://github.com/vapor/vapor.git", .upToNextMajor(...))`
///
/// The name is extracted in priority order:
///   1. Explicit `name:` parameter
///   2. Repository name from the URL path (last path component, `.git` stripped)
pub fn parse_swift_package_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Only look at lines with `.package(`.
        if !trimmed.contains(".package(") {
            continue;
        }

        // Try explicit `name: "..."` first.
        if let Some(name) = extract_swift_string_arg(trimmed, "name:") {
            if is_valid_package_name(&name) {
                packages.push(name);
                continue;
            }
        }

        // Fall back to deriving the name from the URL.
        if let Some(url) = extract_swift_string_arg(trimmed, "url:") {
            if let Some(name) = name_from_url(&url) {
                if is_valid_package_name(&name) {
                    packages.push(name);
                }
            }
        }
    }

    packages
}

/// Extract the string value of a named argument from a Swift function call fragment.
///
/// For `name: "Vapor"` returns `Some("Vapor")`.
/// For `url: "https://github.com/vapor/vapor.git"` returns `Some("https://github.com/vapor/vapor.git")`.
fn extract_swift_string_arg(line: &str, arg_name: &str) -> Option<String> {
    let start = line.find(arg_name)?;
    let after_key = &line[start + arg_name.len()..];
    // Skip whitespace then find the opening quote.
    let after_ws = after_key.trim_start();
    let after_quote = after_ws.strip_prefix('"')?;
    let end = after_quote.find('"')?;
    Some(after_quote[..end].to_string())
}

/// Derive a package name from a Swift package URL.
///
/// `https://github.com/vapor/vapor.git` → `vapor`
/// `https://github.com/apple/swift-argument-parser` → `swift-argument-parser`
fn name_from_url(url: &str) -> Option<String> {
    let last = url.trim_end_matches('/').rsplit('/').next()?;
    let name = last.trim_end_matches(".git");
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn is_valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
}
