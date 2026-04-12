// indexer/manifest/zig_zon.rs — build.zig.zon reader

use std::path::Path;
use super::{ManifestData, ManifestKind, ManifestReader};

pub struct ZigZonManifest;

impl ManifestReader for ZigZonManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::ZigZon }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let zon = project_root.join("build.zig.zon");
        if !zon.is_file() { return None; }
        let content = std::fs::read_to_string(&zon).ok()?;
        let mut data = ManifestData::default();
        for name in parse_zig_zon_deps(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

/// Parse dependency names from build.zig.zon.
///
/// Zig zon uses a Zig struct-literal syntax:
/// ```
/// .dependencies = .{
///     .@"dep_name" = .{ .url = "...", .hash = "..." },
///     .dep_name = .{ ... },
/// },
/// ```
pub fn parse_zig_zon_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_deps = false;
    let mut brace_depth = 0u32;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains(".dependencies") && trimmed.contains("= .{") {
            in_deps = true;
            brace_depth = 1;
            continue;
        }
        if !in_deps { continue; }

        for ch in trimmed.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => {
                    brace_depth = brace_depth.saturating_sub(1);
                    if brace_depth == 0 {
                        in_deps = false;
                    }
                }
                _ => {}
            }
        }

        // Match .@"name" = or .name =
        if brace_depth == 1 {
            if let Some(name) = extract_zon_dep_name(trimmed) {
                if !name.is_empty() {
                    deps.push(name);
                }
            }
        }
    }
    deps
}

fn extract_zon_dep_name(line: &str) -> Option<String> {
    let trimmed = line.trim().trim_start_matches('.');
    // .@"dep-name" = .{ ... }
    if trimmed.starts_with("@\"") {
        let rest = &trimmed[2..];
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    // .dep_name = .{ ... }
    if let Some(eq) = trimmed.find('=') {
        let name = trimmed[..eq].trim();
        if !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Some(name.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_zig_zon() {
        let content = r#"
.{
    .name = .clap,
    .dependencies = .{
        .@"zig-clap" = .{ .url = "...", .hash = "..." },
        .known_folders = .{ .url = "...", .hash = "..." },
    },
}
"#;
        let deps = parse_zig_zon_deps(content);
        assert_eq!(deps, vec!["zig-clap", "known_folders"]);
    }
}
