// indexer/manifest/description.rs — R DESCRIPTION file reader
//
// R packages use a single-file manifest named DESCRIPTION at the package
// root. The format is RFC-822-style key/value pairs — one field per line,
// with continuation lines indented by whitespace:
//
//   Package: dplyr
//   Title: A Grammar of Data Manipulation
//   Imports:
//       cli (>= 3.6.2),
//       generics,
//       glue (>= 1.3.2),
//       lifecycle (>= 1.0.5),
//       magrittr (>= 1.5),
//       rlang (>= 1.1.7),
//       tibble (>= 3.2.0)
//
// The fields we care about for dependency detection are `Depends`,
// `Imports`, `LinkingTo`, and `Suggests`. Each carries a comma-separated
// list of package specifications; each spec is a package name optionally
// followed by a parenthesised version constraint, which we strip.
//
// This reader is consumed by the Phase 1.3 R externals pipeline. R is the
// first language we're wiring up with no prior manifest reader — Phase 0.5's
// bootstrap couldn't initialise R projects on this machine because the
// Rscript toolchain isn't installed, so the DESCRIPTION parser is purely
// the foundation for Phase 3's broader manifest backfill pattern.

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct DescriptionManifest;

impl ManifestReader for DescriptionManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Description
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let description_path = project_root.join("DESCRIPTION");
        if !description_path.is_file() {
            return None;
        }
        let content = std::fs::read_to_string(&description_path).ok()?;

        let mut data = ManifestData::default();
        // For resolver classification, only `Imports` and `Depends` represent
        // packages whose symbols are unconditionally in scope for the package's
        // main R code. `Suggests` and `LinkingTo` are excluded:
        //   - `Suggests`: optional test/vignette deps — not available in runtime
        //     namespaces, so their names must not fire the wildcard import path.
        //   - `LinkingTo`: C/C++ header-level deps — no R symbol imports.
        for name in parse_description_runtime_deps(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse runtime-import package names from a DESCRIPTION file — only
/// `Depends` and `Imports` sections. These are the packages whose symbols
/// are unconditionally in scope for the package's main R code.
///
/// `Suggests` (optional test/vignette deps) and `LinkingTo` (C/C++ headers)
/// are excluded: they must not fire the wildcard import path in the resolver.
///
/// Used by the manifest reader (`DescriptionManifest`) for resolver classification.
/// Use `parse_description_deps` when you need all four sections (e.g. externals
/// discovery, where we want to index Suggests packages if installed).
pub fn parse_description_runtime_deps(content: &str) -> Vec<String> {
    parse_description_fields(content, &["Depends", "Imports"])
}

/// Parse dependency package names from a DESCRIPTION file's body.
///
/// Walks the file line by line and extracts values for the four dependency
/// fields: `Depends`, `Imports`, `LinkingTo`, `Suggests`. Each field's
/// value may span multiple indented continuation lines. The final combined
/// value is split on commas, stripped of version constraints, and added to
/// the result set.
///
/// Returns a `Vec<String>` (deduplicated, stable-order) of package names.
/// For resolver use, prefer `parse_description_runtime_deps` (Imports+Depends only).
pub fn parse_description_deps(content: &str) -> Vec<String> {
    parse_description_fields(content, &["Depends", "Imports", "LinkingTo", "Suggests"])
}

fn parse_description_fields(content: &str, field_names: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for field in field_names {
        if let Some(value) = read_field(content, field) {
            for pkg in split_package_list(&value) {
                if pkg == "R" {
                    // `R (>= 4.1.0)` is a language version pin, not a
                    // package dep. DESCRIPTION dialect uses it as a
                    // bootstrap hint.
                    continue;
                }
                if seen.insert(pkg.clone()) {
                    out.push(pkg);
                }
            }
        }
    }
    out
}

/// Extract the full text of a named DESCRIPTION field, concatenating any
/// indented continuation lines into one string. Field names are matched
/// case-sensitively (DESCRIPTION is not case-insensitive in practice —
/// `Imports:` and `imports:` mean different things to CRAN's lint).
fn read_field(content: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}:");
    let mut iter = content.lines().peekable();
    while let Some(line) = iter.next() {
        if let Some(rest) = line.strip_prefix(&prefix) {
            let mut value = rest.trim().to_string();
            while let Some(next) = iter.peek() {
                // Continuation lines start with whitespace. Any other field
                // ends the current one.
                if next.starts_with(' ') || next.starts_with('\t') {
                    value.push(' ');
                    value.push_str(next.trim());
                    iter.next();
                } else {
                    break;
                }
            }
            return Some(value);
        }
    }
    None
}

/// Split a DESCRIPTION field value on commas and strip version constraints.
///
/// A raw value looks like:
///   `cli (>= 3.6.2), generics, glue (>= 1.3.2)`
///
/// Each comma-separated item may carry a `(> 1.0)`, `(>= 1.0)`, `(== 1.0)`,
/// or similar version tail in parentheses. We drop the parenthesised tail
/// and keep the bare package name.
fn split_package_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|item| {
            let trimmed = item.trim();
            // Strip version constraint if present.
            let name = match trimmed.find('(') {
                Some(idx) => trimmed[..idx].trim(),
                None => trimmed,
            };
            name.to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Package: dplyr
Title: A Grammar of Data Manipulation
Version: 1.2.1.9000
Depends:
    R (>= 4.1.0)
Imports:
    cli (>= 3.6.2),
    generics,
    glue (>= 1.3.2),
    lifecycle (>= 1.0.5),
    magrittr (>= 1.5),
    methods,
    pillar (>= 1.9.0),
    R6,
    rlang (>= 1.1.7),
    tibble (>= 3.2.0),
    tidyselect (>= 1.2.0),
    utils,
    vctrs (>= 0.7.1)
Suggests:
    broom,
    covr,
    DBI,
    ggplot2
LinkingTo:
    cpp11 (>= 0.4.7)
License: MIT + file LICENSE
";

    #[test]
    fn parses_dplyr_description_fields() {
        let deps = parse_description_deps(SAMPLE);
        // Imports
        assert!(deps.contains(&"cli".to_string()));
        assert!(deps.contains(&"generics".to_string()));
        assert!(deps.contains(&"glue".to_string()));
        assert!(deps.contains(&"lifecycle".to_string()));
        assert!(deps.contains(&"rlang".to_string()));
        assert!(deps.contains(&"tibble".to_string()));
        assert!(deps.contains(&"vctrs".to_string()));
        // Suggests
        assert!(deps.contains(&"broom".to_string()));
        assert!(deps.contains(&"ggplot2".to_string()));
        // LinkingTo
        assert!(deps.contains(&"cpp11".to_string()));
    }

    #[test]
    fn strips_r_version_pin() {
        // The `R (>= 4.1.0)` in Depends must not leak into the dep set.
        let deps = parse_description_deps(SAMPLE);
        assert!(!deps.contains(&"R".to_string()));
    }

    #[test]
    fn strips_version_constraints() {
        let deps = parse_description_deps("Imports:\n    foo (>= 1.0),\n    bar (== 2.0),\n    baz\n");
        assert_eq!(deps, vec!["foo".to_string(), "bar".to_string(), "baz".to_string()]);
    }

    #[test]
    fn deduplicates_across_fields() {
        let deps = parse_description_deps(
            "Imports:\n    foo\nSuggests:\n    foo,\n    bar\n",
        );
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&"foo".to_string()));
        assert!(deps.contains(&"bar".to_string()));
    }

    #[test]
    fn ignores_non_dependency_fields() {
        let deps = parse_description_deps(
            "Package: dplyr\nTitle: Things\nVersion: 1.0\nImports:\n    foo\n",
        );
        assert_eq!(deps, vec!["foo".to_string()]);
    }
}
