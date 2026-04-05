// indexer/manifest/mix.rs — mix.exs reader (Elixir)

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct MixManifest;

impl ManifestReader for MixManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Mix
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let mix_exs_path = project_root.join("mix.exs");
        if !mix_exs_path.is_file() {
            return None;
        }
        let content = std::fs::read_to_string(&mix_exs_path).ok()?;

        let mut data = ManifestData::default();
        for name in parse_mix_deps(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

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
