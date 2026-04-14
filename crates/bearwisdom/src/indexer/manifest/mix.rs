// indexer/manifest/mix.rs — mix.exs reader (Elixir)

use std::path::{Path, PathBuf};

use super::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

pub struct MixManifest;

impl ManifestReader for MixManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Mix
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
        collect_mix_files(project_root, &mut paths, 0);

        let mut out = Vec::new();
        for manifest_path in paths {
            let Ok(content) = std::fs::read_to_string(&manifest_path) else { continue };

            let mut data = ManifestData::default();
            for name in parse_mix_deps(&content) {
                data.dependencies.insert(name);
            }

            let name = parse_mix_app_name(&content);
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

fn collect_mix_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
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
                ".git" | "deps" | "_build" | "node_modules" | "target" | "bin" | "obj"
            ) {
                continue;
            }
            collect_mix_files(&path, out, depth + 1);
        } else if entry.file_name() == "mix.exs" {
            out.push(path);
        }
    }
}

/// Parse the OTP app name from `app: :my_app` in the `project/0` function.
fn parse_mix_app_name(content: &str) -> Option<String> {
    // Match `app: :name` where `name` is an Elixir atom.
    let needle = "app:";
    let mut search_from = 0;
    while let Some(pos) = content[search_from..].find(needle) {
        let abs = search_from + pos;
        let after = &content[abs + needle.len()..];
        let after = after.trim_start();
        if let Some(rest) = after.strip_prefix(':') {
            // Atom name ends at `,`, `}`, whitespace, or EOL.
            let end = rest
                .find(|c: char| c == ',' || c == '}' || c.is_whitespace())
                .unwrap_or(rest.len());
            let name = rest[..end].trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
        search_from = abs + needle.len();
    }
    None
}

/// Parse dependency names from a `mix.exs` file.
///
/// mix.exs uses Elixir tuple syntax inside a `deps/0` function:
/// ```elixir
/// defp deps do
///   [
///     {:phoenix, "~> 1.7"},
///     {:ecto_sql, "~> 3.10"},
///     {:postgrex, ">= 0.0.0"},
///     {:plug_cowboy, "~> 2.5", only: :prod},
///   ]
/// end
/// ```
///
/// The atom name is the first element of each tuple: `{:dep_name, ...}`.
/// Line-based; no Elixir parser needed.
pub fn parse_mix_deps(content: &str) -> Vec<String> {
    let mut packages = Vec::new();
    let mut in_deps_fn = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comment lines.
        if trimmed.starts_with('#') {
            continue;
        }

        // Detect entry into `deps` function body.
        // Matches: `defp deps do`, `def deps do`, `defp deps(_) do`.
        if trimmed.starts_with("defp deps") || trimmed.starts_with("def deps") {
            in_deps_fn = true;
            continue;
        }

        // Detect end of function — `end` at the same or lower indentation.
        // We use a simple heuristic: bare `end` exits.
        if in_deps_fn && trimmed == "end" {
            in_deps_fn = false;
            continue;
        }

        if !in_deps_fn {
            continue;
        }

        // Look for tuple entries: `{:dep_name, ...}` or `{:dep_name, version, opts}`.
        if let Some(name) = extract_mix_dep_atom(trimmed) {
            if !name.is_empty()
                && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                packages.push(name);
            }
        }
    }

    packages
}

/// Extract the atom name from a mix dependency tuple line.
///
/// Handles:
///   `{:phoenix, "~> 1.7"}` → `"phoenix"`
///   `  {:ecto_sql, "~> 3.10"},` → `"ecto_sql"`
fn extract_mix_dep_atom(line: &str) -> Option<String> {
    // Find `{:` which starts a dep tuple.
    let start = line.find("{:")?;
    let after_colon = &line[start + 2..];
    // The atom name ends at `,`, `}`, or whitespace.
    let end = after_colon
        .find(|c: char| c == ',' || c == '}' || c.is_whitespace())
        .unwrap_or(after_colon.len());
    let name = after_colon[..end].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}
