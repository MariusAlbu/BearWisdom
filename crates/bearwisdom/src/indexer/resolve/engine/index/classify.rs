// =============================================================================
// indexer/resolve/engine/index/classify.rs — external-classification queries
//
// SymbolIndex methods that answer "is this path / name external?" questions.
// Distinct from the `SymbolLookup` trait impl: these are higher-level
// queries that combine multiple internal lookups (tsconfig types,
// `@types/*` paths, npm re-export chains, language primitives), not direct
// field reads.
// =============================================================================

use std::collections::HashSet;

use super::super::{file_belongs_to_npm_package, npm_package_from_specifier};
use super::SymbolIndex;
use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};

impl SymbolIndex {
    /// Test whether a file path belongs to a package listed in any
    /// project's `tsconfig.json#compilerOptions.types`. Such packages
    /// supply ambient globals — symbols available without an `import`
    /// statement — so the heuristic resolver should prefer their
    /// candidates when disambiguating bare-name refs.
    ///
    /// `tsconfig_types_union` entries are raw values like
    /// `"vitest/globals"`, `"node"`, `"@types/jest"`. We match by
    /// asking whether the candidate path contains
    /// `node_modules/<name>/...` for any name in the list. Slash
    /// inside an entry means a sub-path inside the package — we still
    /// match against the package root since the whole package is
    /// pulled in by the types declaration.
    pub fn is_ambient_path(&self, path: &str) -> bool {
        let lower = path.to_lowercase();

        // 1. Packages explicitly listed in `tsconfig.json#compilerOptions.types`
        //    — the project opted into these as ambient providers.
        for entry in &self.tsconfig_types_union {
            // Take the package portion: `vitest/globals` → `vitest`.
            // Scoped packages (`@scope/pkg/sub`) keep both segments:
            // `@scope/pkg/sub` → `@scope/pkg`.
            let pkg_root = if let Some(stripped) = entry.strip_prefix('@') {
                let mut parts = stripped.splitn(3, '/');
                match (parts.next(), parts.next()) {
                    (Some(scope), Some(name)) => format!("@{scope}/{name}"),
                    _ => entry.clone(),
                }
            } else {
                entry.split('/').next().unwrap_or(entry).to_string()
            };
            let needle = format!("node_modules/{}/", pkg_root.to_lowercase());
            if lower.contains(&needle) {
                return true;
            }
        }

        // 2. `@types/*` packages — TypeScript auto-includes these as
        //    ambient by default when `compilerOptions.types` is not
        //    explicitly set, and most projects don't override that.
        //    `@types/node` provides `process` / `Buffer`, `@types/jest`
        //    provides `expect` / `describe`, etc. Match the path suffix
        //    so per-package nesting (`@types/node/fs.d.ts`) is covered.
        if lower.contains("/@types/") || lower.contains("node_modules/@types/") {
            return true;
        }

        // 3. Convention fallback: any file literally named `globals.d.ts`.
        //    Some packages (e.g. `@playwright/test`, `@vitest/runner`)
        //    distribute their declare-global block here regardless of
        //    whether they're in @types or in the project's types list.
        if lower.ends_with("/globals.d.ts") || lower.ends_with("\\globals.d.ts") {
            return true;
        }

        false
    }

    /// Walk the cross-package re-export chain starting from `module_path`
    /// (a bare specifier the user imported `target_name` from), looking
    /// for a candidate file that actually owns `target_name`.
    ///
    /// Why: many npm libraries split a public package from one or more
    /// internal packages. A user file does
    ///   `import { TSESTree } from '@typescript-eslint/utils'`
    /// but the actual definition lives in `@typescript-eslint/types` (the
    /// public package re-exports `TSESTree` via `export { TSESTree } from
    /// '@typescript-eslint/types'`). Priority 1's path-match alone fails
    /// because the candidate's path is in a different package than the
    /// import specifier. Walking the chain catches the right one.
    ///
    /// Bounded by depth 4 — covers the common 1–3 hop chains without
    /// opening up arbitrary search space. Visited set on the package
    /// frontier prevents cycles.
    ///
    /// Returns the first candidate id found in any reachable package, or
    /// None when the chain doesn't lead to a known symbol.
    pub fn resolve_via_external_reexport(
        &self,
        suffix: &str,
        prefix: &str,
        module_path: &str,
        _heuristic_candidates: &[(String, String, String, i64)],
    ) -> Option<i64> {
        if module_path.is_empty() { return None; }
        let importing_pkg = npm_package_from_specifier(module_path)?;
        // We query SymbolIndex's full by_name index here rather than the
        // heuristic's name_to_ids — the heuristic filters out most
        // external d.ts files (only `@types/*` and TS stdlib survive),
        // so transitive re-export targets like `@typescript-eslint/types`
        // would be invisible to it. The chain walker needs the complete
        // external symbol set.
        let candidates = self.by_name(suffix);
        if candidates.is_empty() { return None; }
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut frontier: Vec<String> = vec![importing_pkg.clone()];
        visited.insert(importing_pkg);
        for _depth in 0..4 {
            for pkg in &frontier {
                for sym in candidates {
                    if file_belongs_to_npm_package(&sym.file_path, pkg) {
                        return Some(sym.id);
                    }
                }
            }
            let mut next: Vec<String> = Vec::new();
            for pkg in &frontier {
                let Some(reexports) = self.pkg_reexports.get(pkg) else { continue };
                for (name, target_pkg) in reexports {
                    if name != "*" && name != prefix { continue }
                    if visited.insert(target_pkg.clone()) {
                        next.push(target_pkg.clone());
                    }
                }
            }
            if next.is_empty() { break }
            frontier = next;
        }
        let _ = suffix; let _ = prefix; // silence if unused
        None
    }
    /// Classify an external name with its specific namespace category.
    ///
    /// Returns:
    ///   - `Some("primitive")` for language keyword types (int, string, bool)
    ///   - `Some("builtin")` for runtime globals (console, print, len, Array)
    ///   - `None` if the name is not classified as external
    pub fn classify_external_name(&self, name: &str, language: &str) -> Option<&'static str> {
        // Check the merged external set (primitives + query builtins).
        if let Some(all_externals) = self.primitives_by_language.get(language) {
            if all_externals.contains(name) {
                // Distinguish: plugin.keywords() are "primitive", everything
                // else (query builtins) is "builtin".
                let plugin_keywords =
                    crate::indexer::keywords::keywords_for_language(language);
                if plugin_keywords.contains(&name) {
                    return Some("primitive");
                }
                return Some("builtin");
            }
        }

        None
    }
}
