// =============================================================================
// ecosystem/npm/subpath_entries.rs — deep entry points a package publishes
// under its own module specifier
// =============================================================================

use std::path::PathBuf;

use crate::ecosystem::externals::ExternalDepRoot;

use super::walk::extract_types_from_conditions;

/// Whether a demanded subpath can be probed as an on-disk entry: every segment
/// must be a real name, so a specifier carrying `.`/`..` segments (or an empty
/// one) never escapes the dep root.
///
/// Nesting is not a disqualifier — `legacy/image` and `font/google` are flat-
/// file entry shapes exactly like `server`.
fn probeable_subpath(rest: &str) -> bool {
    !rest.is_empty()
        && rest
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// Resolve each CONCRETE subpath export of a package to its `.d.ts` entry.
///
/// A package's `exports` map publishes deep entry points the user imports
/// directly — `import { useState } from 'preact/hooks'`, `import { ajax } from
/// 'rxjs/ajax'`. Each concrete `"./sub"` key carries its own `types` condition
/// pointing at a separate declaration file the package-root entry never
/// re-exports. Returns `(subpath_suffix, entry_path)` for every concrete key,
/// where `subpath_suffix` is the key without its leading `.` (`"./hooks"` →
/// `"/hooks"`) so the caller forms the import specifier as `module + suffix`.
///
/// The `"."` root key (handled by `resolve_package_entry_path`) and wildcard
/// patterns (`"./*"`, which name no single file) are skipped — only hand-
/// declared concrete entry points, so the set stays bounded to the package's
/// published API surface.
///
/// Packages that declare no `exports` map still publish deep entries as plain
/// sibling declaration files resolved by TypeScript's classic file lookup;
/// those are probed from the root's demanded specifiers, so the set stays
/// bounded to what the workspace actually imports.
pub(crate) fn resolve_package_subpath_entries(dep: &ExternalDepRoot) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    // `exports`-map subpaths — authoritative when the package declares them.
    if let Ok(json_str) = std::fs::read_to_string(dep.root.join("package.json")) {
        if let Ok(pj) = serde_json::from_str::<serde_json::Value>(&json_str) {
            if let Some(exports) = pj.get("exports").and_then(|e| e.as_object()) {
                for (key, cond) in exports {
                    if key == "." || !key.starts_with("./") || key.contains('*') {
                        continue;
                    }
                    let Some(rel) = extract_types_from_conditions(cond) else {
                        continue;
                    };
                    let entry = dep.root.join(rel.trim_start_matches("./"));
                    if entry.is_file() {
                        out.push((key.strip_prefix('.').unwrap_or(key).to_string(), entry));
                    }
                }
            }
        }
    }
    // Flat-file subpaths the project imports that no `exports` map declares: a
    // package may publish `pkg/sub` as a sibling `<root>/sub.d.ts` resolved by
    // TypeScript's classic file lookup. Probe only the demanded subpaths
    // (`dep.requested_imports`) so the set stays bounded to the project's
    // actual API surface — not every `.d.ts` under the package root.
    for spec in &dep.requested_imports {
        let Some(rest) = spec
            .strip_prefix(dep.module_path.as_str())
            .and_then(|r| r.strip_prefix('/'))
        else {
            continue;
        };
        if !probeable_subpath(rest) {
            continue;
        }
        let suffix = format!("/{rest}");
        if out.iter().any(|(s, _)| *s == suffix) {
            continue;
        }
        for cand in [
            dep.root.join(format!("{rest}.d.ts")),
            dep.root.join(format!("{rest}.d.mts")),
            dep.root.join(format!("{rest}.d.cts")),
            dep.root.join(rest).join("index.d.ts"),
        ] {
            if cand.is_file() {
                out.push((suffix.clone(), cand));
                break;
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "subpath_entries_tests.rs"]
mod tests;
