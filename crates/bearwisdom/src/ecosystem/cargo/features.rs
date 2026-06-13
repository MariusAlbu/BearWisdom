// ---------------------------------------------------------------------------
// Per-crate enabled-feature sets, parsed from the project's Cargo.toml files
// ---------------------------------------------------------------------------
//
// Cargo.lock (v3/v4) does not serialize a per-package resolved feature set —
// it records only the dependency graph. The feature information that *is*
// readable as project-declared data lives in each manifest's dependency
// tables: `windows = { version = "0.61", features = ["Win32_Foundation"] }`.
//
// Discovery scans every `Cargo.toml` in the project, unions the declared
// feature arrays per crate name across all packages (a monorepo may declare
// the same crate with different features in different members), and registers
// the result against each discovered crate root. The cfg-gated module walk
// (`reachability::expand_rust_mods_into`) reads this map to decide which
// `#[cfg(feature = "X")] pub mod X;` subtrees to descend.
//
// A crate with no collected feature set keeps an empty entry, which the walk
// treats as FAIL OPEN — every module is walked, exactly as before gating.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Process-wide map: canonical crate-root path → enabled feature set. Mirrors
/// the `COURSIER_INDEX` memo pattern in `externals.rs`. Populated by
/// `register_root_features` during discovery (which always runs before the
/// walk phase within an indexing run) and read by the reachability walk.
static ROOT_FEATURES: OnceLock<Mutex<HashMap<PathBuf, Vec<String>>>> = OnceLock::new();

fn root_features_map() -> &'static Mutex<HashMap<PathBuf, Vec<String>>> {
    ROOT_FEATURES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn canonical_key(root: &Path) -> PathBuf {
    std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
}

/// Record the enabled feature set for one discovered crate root. Replaces any
/// prior entry for the same root so a re-index reflects the current manifests.
pub(super) fn register_root_features(root: &Path, features: Vec<String>) {
    let mut guard = root_features_map().lock().expect("cargo feature map poisoned");
    guard.insert(canonical_key(root), features);
}

/// Enabled feature set for a crate root, or an empty vec when none was
/// registered (FAIL OPEN — the walk descends every module).
pub(super) fn enabled_features_for_root(root: &Path) -> Vec<String> {
    let guard = root_features_map().lock().expect("cargo feature map poisoned");
    guard.get(&canonical_key(root)).cloned().unwrap_or_default()
}

/// Build a `crate_name → unioned enabled features` map from every `Cargo.toml`
/// under `project_root`. Pure parse over the supplied manifest contents; the
/// caller resolves crate names to on-disk roots.
pub(super) fn collect_crate_features(project_root: &Path) -> HashMap<String, Vec<String>> {
    let mut tomls = Vec::new();
    collect_cargo_tomls(project_root, &mut tomls, 0);

    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for path in tomls {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (name, features) in parse_dependency_features(&content) {
            let entry = map.entry(name).or_default();
            for f in features {
                if !entry.contains(&f) {
                    entry.push(f);
                }
            }
        }
    }
    map
}

/// Walk the project tree collecting `Cargo.toml` paths, skipping build-output
/// and VCS directories. Bound mirrors the manifest reader's own walk.
fn collect_cargo_tomls(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "target" | ".git" | "node_modules" | "bin" | "obj" | ".cargo"
                ) {
                    continue;
                }
            }
            collect_cargo_tomls(&path, out, depth + 1);
        } else if entry.file_name() == "Cargo.toml" {
            out.push(path);
        }
    }
}

/// Parse `(crate_name, features)` pairs from a single manifest's dependency
/// tables. Recognizes both forms that carry a `features = [...]` array:
///
///   * inline table — `windows = { version = "0.61", features = ["A", "B"] }`,
///     including the multi-line variant the spec fixture uses;
///   * sub-table    — `[dependencies.windows]\nfeatures = ["A", "B"]`.
///
/// A dependency with no `features` array yields no pair. Crate names are the
/// dependency KEY (the `use`-side name), matching how roots are keyed.
pub(super) fn parse_dependency_features(content: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();

    // Sub-table state: `[dependencies.<name>]` opens a block whose body may
    // hold `features = [...]` spread over lines.
    let mut subtable_name: Option<String> = None;
    let mut subtable_features: Vec<String> = Vec::new();

    // Inline-table state: an opened `name = {` whose `}` hasn't been seen yet.
    let mut inline_name: Option<String> = None;
    let mut inline_buf = String::new();

    let mut in_dep_section = false;

    let flush_subtable =
        |name: &mut Option<String>, feats: &mut Vec<String>, out: &mut Vec<(String, Vec<String>)>| {
            if let Some(n) = name.take() {
                if !feats.is_empty() {
                    out.push((n, std::mem::take(feats)));
                }
            }
            feats.clear();
        };

    for raw in content.lines() {
        let trimmed = raw.trim();

        // Continue accumulating an open inline table across lines.
        if let Some(name) = inline_name.clone() {
            inline_buf.push(' ');
            inline_buf.push_str(trimmed);
            if trimmed.contains('}') {
                let feats = parse_features_array(&inline_buf);
                if !feats.is_empty() {
                    out.push((name, feats));
                }
                inline_name = None;
                inline_buf.clear();
            }
            continue;
        }

        if trimmed.starts_with('[') {
            // A new table header ends any open sub-table block.
            flush_subtable(&mut subtable_name, &mut subtable_features, &mut out);
            if let Some(name) = subtable_dep_name(trimmed) {
                subtable_name = Some(name);
                in_dep_section = false;
            } else {
                in_dep_section = is_dependency_section(trimmed);
            }
            continue;
        }

        if subtable_name.is_some() {
            if let Some(feats) = features_line(trimmed) {
                subtable_features = feats;
            }
            continue;
        }

        if !in_dep_section || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some(eq) = trimmed.find('=') else {
            continue;
        };
        let key = trimmed[..eq].trim().split('.').next().unwrap_or("").trim();
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            continue;
        }
        let value = trimmed[eq + 1..].trim();
        if value.starts_with('{') {
            if value.contains('}') {
                // Single-line inline table.
                let feats = parse_features_array(value);
                if !feats.is_empty() {
                    out.push((key.to_string(), feats));
                }
            } else {
                // Inline table opened on this line, continues below.
                inline_name = Some(key.to_string());
                inline_buf.clear();
                inline_buf.push_str(value);
            }
        }
    }

    // EOF: flush any trailing sub-table block.
    flush_subtable(&mut subtable_name, &mut subtable_features, &mut out);
    out
}

/// Extract a `features = [ "a", "b" ]` array from a buffer (an inline table
/// body or a standalone line). Returns the listed feature names; empty when no
/// `features` key is present.
fn parse_features_array(buf: &str) -> Vec<String> {
    let Some(idx) = buf.find("features") else {
        return Vec::new();
    };
    let after = buf[idx + "features".len()..].trim_start();
    let Some(after) = after.strip_prefix('=') else {
        return Vec::new();
    };
    let after = after.trim_start();
    let Some(after) = after.strip_prefix('[') else {
        return Vec::new();
    };
    let end = after.find(']').unwrap_or(after.len());
    split_string_array(&after[..end])
}

/// A standalone `features = [...]` line inside a `[dependencies.<name>]`
/// sub-table. Returns the array contents when the line opens AND closes the
/// array; multi-line sub-table arrays are uncommon and fall through to empty.
fn features_line(line: &str) -> Option<Vec<String>> {
    let rest = line.strip_prefix("features")?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('[')?;
    let end = rest.find(']')?;
    Some(split_string_array(&rest[..end]))
}

/// Split a comma-separated list of double-quoted strings, ignoring whitespace
/// and trailing commas: `"a", "b" ,` → `["a", "b"]`.
fn split_string_array(body: &str) -> Vec<String> {
    body.split(',')
        .filter_map(|part| {
            let p = part.trim();
            p.strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        })
        .collect()
}

/// True for a flat dependency table header (`[dependencies]`,
/// `[target.'cfg(windows)'.dependencies]`, etc.). Mirrors the manifest
/// reader's classification.
fn is_dependency_section(trimmed: &str) -> bool {
    matches!(
        trimmed,
        "[dependencies]"
            | "[dev-dependencies]"
            | "[build-dependencies]"
            | "[workspace.dependencies]"
            | "[workspace.dev-dependencies]"
            | "[workspace.build-dependencies]"
    ) || (trimmed.starts_with("[target.")
        && (trimmed.ends_with(".dependencies]")
            || trimmed.ends_with(".dev-dependencies]")
            || trimmed.ends_with(".build-dependencies]")))
}

/// Extract the crate name from a sub-table header
/// (`[dependencies.windows]`, `[target.'cfg(windows)'.dependencies.windows]`).
fn subtable_dep_name(trimmed: &str) -> Option<String> {
    let body = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    for marker in [
        "workspace.dependencies.",
        "workspace.dev-dependencies.",
        "workspace.build-dependencies.",
        "dev-dependencies.",
        "build-dependencies.",
        "dependencies.",
    ] {
        if let Some((_, name)) = body.rsplit_once(marker) {
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                return Some(name.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
pub(super) fn _test_collect_crate_features(root: &Path) -> HashMap<String, Vec<String>> {
    collect_crate_features(root)
}

#[cfg(test)]
#[path = "features_tests.rs"]
mod tests;
