// indexer/manifest/cargo.rs — Cargo.toml reader

use std::path::{Path, PathBuf};

use super::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

pub struct CargoManifest;

impl ManifestReader for CargoManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Cargo
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let entries = self.read_all(project_root);
        if entries.is_empty() {
            return None;
        }
        let mut data = ManifestData::default();
        for e in &entries {
            data.dependencies.extend(e.data.dependencies.iter().cloned());
        }
        Some(data)
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let mut paths = Vec::new();
        collect_cargo_tomls(project_root, &mut paths, 0);

        let mut out = Vec::new();
        for manifest_path in paths {
            let Ok(content) = std::fs::read_to_string(&manifest_path) else { continue };

            let mut data = ManifestData::default();
            for name in parse_cargo_dependencies(&content) {
                data.dependencies.insert(name);
            }
            // Path deps are sibling workspace crates — surface them as
            // project_refs so workspace_graph shows manifest-declared
            // intent even when the root has no [workspace.members].
            for key in parse_cargo_path_dependencies(&content) {
                if !data.project_refs.contains(&key) {
                    data.project_refs.push(key);
                }
            }

            let name = parse_cargo_package_name(&content);
            let package_dir = manifest_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| project_root.to_path_buf());

            out.push(ReaderEntry {
                package_dir,
                manifest_path,
                data,
                name,
            });
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

pub(super) fn collect_cargo_tomls(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 {
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
                "target" | ".git" | "node_modules" | "bin" | "obj" | ".cargo"
            ) {
                continue;
            }
            collect_cargo_tomls(&path, out, depth + 1);
        } else if entry.file_name() == "Cargo.toml" {
            out.push(path);
        }
    }
}

/// Parse crate names from `[dependencies]` and `[dev-dependencies]` sections.
///
/// TOML parsing is done line-by-line to avoid pulling in a full TOML crate.
/// We only need crate names (keys), not version strings.
///
/// Handles:
///   `serde = "1"`
///   `tokio = { version = "1", features = ["full"] }`
///   `my-crate.workspace = true`
pub fn parse_cargo_dependencies(content: &str) -> Vec<String> {
    let mut crates = Vec::new();
    let mut in_dep_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect section headers.
        if trimmed.starts_with('[') {
            in_dep_section = matches!(
                trimmed,
                "[dependencies]"
                    | "[dev-dependencies]"
                    | "[build-dependencies]"
                    | "[workspace.dependencies]"
            );
            continue;
        }

        if !in_dep_section {
            continue;
        }

        // Skip blank lines and comments.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Extract the crate name (the key before `=`).
        // Keys may contain hyphens and underscores but not spaces.
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos]
                .trim()
                // Strip dotted suffixes like `tokio.workspace`
                .split('.')
                .next()
                .unwrap_or("")
                .trim();
            if !key.is_empty()
                && key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                crates.push(key.to_string());
            }
        }
    }

    crates
}

/// Parse sibling-workspace crate names from `path = "..."` dependency entries.
///
/// Covers `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`,
/// `[workspace.dependencies]`. For each dep line containing `path = "..."`,
/// emits the key (the import-side name Cargo uses) so the resolver can match
/// it against a sibling package's `declared_name`.
///
/// The Cargo key IS what appears in `use foo::...` source code — even when
/// the target crate's `[package].name` differs (aliased deps). The key has
/// higher resolution value than the path's folder stem.
///
/// Multi-line inline tables are handled heuristically by checking whether a
/// line without a `path =` contains `{` (start of an inline table) and
/// retaining the key until a closing `}` line appears.
pub fn parse_cargo_path_dependencies(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_dep_section = false;
    let mut pending_key: Option<String> = None;
    let mut pending_table = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            in_dep_section = matches!(
                trimmed,
                "[dependencies]"
                    | "[dev-dependencies]"
                    | "[build-dependencies]"
                    | "[workspace.dependencies]"
            );
            pending_key = None;
            pending_table.clear();
            continue;
        }
        if !in_dep_section {
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Multi-line inline-table continuation.
        if let Some(key) = pending_key.clone() {
            pending_table.push(' ');
            pending_table.push_str(trimmed);
            if trimmed.contains('}') {
                if pending_table.contains("path") && pending_table.contains('=') {
                    if !out.contains(&key) {
                        out.push(key);
                    }
                }
                pending_key = None;
                pending_table.clear();
            }
            continue;
        }

        let Some(eq) = trimmed.find('=') else { continue };
        let key = trimmed[..eq]
            .trim()
            .split('.')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if key.is_empty()
            || !key.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            continue;
        }
        let value = trimmed[eq + 1..].trim();
        // Inline table on the same line.
        if value.starts_with('{') && value.ends_with('}') {
            if value.contains("path") && value.contains('=') {
                if !out.contains(&key) {
                    out.push(key);
                }
            }
            continue;
        }
        // Inline table spanning multiple lines.
        if value.starts_with('{') {
            pending_key = Some(key);
            pending_table.push_str(value);
            continue;
        }
        // Bare-version form (`serde = "1"`) — never a path dep.
    }

    out
}

/// Parse the `[package].name` field from a Cargo.toml.
///
/// Returns `None` for workspace-root manifests that declare no `[package]`
/// section (pure `[workspace]` manifests).
fn parse_cargo_package_name(content: &str) -> Option<String> {
    let mut in_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        // `name = "crate-name"` — take the key before `=`, value between quotes.
        if let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('=') else { continue };
            let rest = rest.trim();
            let Some(rest) = rest.strip_prefix('"') else { continue };
            let Some(end) = rest.find('"') else { continue };
            return Some(rest[..end].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_deps_inline_table_single_line() {
        let toml = r#"
[dependencies]
serde = "1"
core = { path = "../core" }
tokio = { version = "1", features = ["full"] }
"#;
        let paths = parse_cargo_path_dependencies(toml);
        assert_eq!(paths, vec!["core"]);
    }

    #[test]
    fn path_deps_multi_line_inline_table() {
        let toml = r#"
[dependencies]
shared = {
    path = "../shared",
    version = "0.1"
}
remote = { version = "1" }
"#;
        let paths = parse_cargo_path_dependencies(toml);
        assert_eq!(paths, vec!["shared"]);
    }

    #[test]
    fn path_deps_across_multiple_dep_sections() {
        let toml = r#"
[dependencies]
core = { path = "../core" }

[dev-dependencies]
testutil = { path = "../testutil" }

[build-dependencies]
builder = { path = "../builder" }
"#;
        let paths = parse_cargo_path_dependencies(toml);
        assert!(paths.contains(&"core".to_string()));
        assert!(paths.contains(&"testutil".to_string()));
        assert!(paths.contains(&"builder".to_string()));
    }

    #[test]
    fn path_deps_ignores_registry_entries() {
        let toml = r#"
[dependencies]
serde = "1"
tokio = { version = "1" }
anyhow = "1.0"
"#;
        let paths = parse_cargo_path_dependencies(toml);
        assert!(paths.is_empty());
    }
}
