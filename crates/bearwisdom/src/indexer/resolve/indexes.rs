// =============================================================================
// indexer/resolve/indexes.rs — side-table builders consumed by the resolve loop
//
// The resolve loop precomputes a handful of lookups from the parsed-file
// stream before running tier-1 and tier-2: a simple-name index, a qname
// index, a per-file namespace map, a module-stem-to-files map, and a per-file
// import map. These were historically attached to the heuristic resolver
// because the heuristic was the only consumer; they are now side-tables to
// the whole resolve pass.
// =============================================================================

use std::collections::HashMap;

use rustc_hash::FxHashMap;

use crate::types::{EdgeKind, ParsedFile, SymbolKind};

use super::path_util::is_external_path;

pub(super) fn build_name_index(
    symbol_id_map: &HashMap<(String, String), i64>,
    parsed: &[ParsedFile],
) -> FxHashMap<String, Vec<(String, String, String, i64)>> {
    // Build a secondary map from (file, qname) → kind string using parsed data.
    // External files are skipped — they belong in SymbolIndex for Tier 1 lookup
    // only, not in the heuristic fallback path (their symbols would pollute
    // cross-language name lookups, e.g. Python `get` matching a TS `get` ref).
    //
    // Narrow exception: external `.d.ts` files that declare runtime-global
    // names user code invokes without imports —
    //   1. `declare global { ... }` carriers (vitest globals, @types/jest
    //      globals, @types/node globals).
    //   2. TypeScript stdlib declaration bundles (`lib.dom.d.ts`,
    //      `lib.es*.d.ts`). DOM methods (`querySelector`, `dispatchEvent`,
    //      `getAttribute`), Function prototype methods (`bind`, `apply`,
    //      `call`), and ECMAScript built-ins all live there and are
    //      callable on any object without an import.
    //
    // Including these lets Priority 4 resolve DOM/ES calls, and Priority
    // 3.8 resolve declare-global names (`expect`/`describe`/etc.).
    fn keep_for_heuristic(path: &str) -> bool {
        if !is_external_path(path) {
            return true;
        }
        let lower = path.to_lowercase();
        if lower.ends_with("/globals.d.ts") || lower.ends_with("globals.d.ts") {
            return true;
        }
        // TypeScript stdlib — path contains `typescript/lib/lib.` (no
        // leading slash, because the virtual-path prefix is `ext:ts:` not
        // a filesystem path). Covers lib.dom.d.ts, lib.es5.d.ts,
        // lib.es2015.core.d.ts, etc.
        if lower.contains("typescript/lib/lib.") && lower.ends_with(".d.ts") {
            return true;
        }
        // DefinitelyTyped packages — `@types/chai`, `@types/mocha`,
        // `@types/jest`, `@types/sinon`. These declare types + methods
        // whose chain roots are often globals (`expect(x).to.equal()` —
        // chai's Assertion.equal). Keeping them in name_to_ids lets the
        // heuristic resolve bare-name matcher calls (`equal`, `eql`)
        // without full chain-type inference. Framework-native packages
        // (`@vitest/*`, `@jest/*`) should come through via npm transitive
        // discovery, not heuristic; this filter stays narrow.
        lower.contains("@types/") && lower.ends_with(".d.ts")
    }

    let mut kind_map: FxHashMap<(&str, &str), &str> = FxHashMap::default();
    for pf in parsed {
        if !keep_for_heuristic(&pf.path) {
            continue;
        }
        for sym in &pf.symbols {
            kind_map.insert((pf.path.as_str(), sym.qualified_name.as_str()), sym.kind.as_str());
        }
    }

    let mut map: FxHashMap<String, Vec<(String, String, String, i64)>> = FxHashMap::default();
    for ((file, qname), &id) in symbol_id_map {
        if !keep_for_heuristic(file) {
            continue;
        }
        // Extract the simple name (last segment of the qualified name).
        let simple = qname.rsplit('.').next().unwrap_or(qname.as_str()).to_string();
        let kind = kind_map
            .get(&(file.as_str(), qname.as_str()))
            .copied()
            .unwrap_or("")
            .to_string();
        map.entry(simple)
            .or_default()
            .push((file.clone(), qname.clone(), kind, id));
    }
    map
}

/// Build a map from qualified_name → symbol_id for exact dotted-path matches.
pub(super) fn build_qname_index(
    symbol_id_map: &HashMap<(String, String), i64>,
) -> FxHashMap<String, i64> {
    symbol_id_map
        .iter()
        .filter(|((file, _), _)| !is_external_path(file))
        .map(|((_, qname), &id)| (qname.clone(), id))
        .collect()
}

/// Build a map from file_path → namespace for same-namespace resolution.
///
/// For each file, finds the first `Namespace` symbol and records its
/// qualified name as the file's namespace.
pub(super) fn build_file_namespace_map(parsed: &[ParsedFile]) -> FxHashMap<String, String> {
    let mut map = FxHashMap::default();
    for pf in parsed {
        if is_external_path(&pf.path) {
            continue;
        }
        if let Some(ns_sym) = pf.symbols.iter().find(|s| s.kind == SymbolKind::Namespace) {
            map.insert(pf.path.clone(), ns_sym.qualified_name.clone());
        }
    }
    map
}

/// Build a reverse index: module/namespace name → file paths.
///
/// Two sources:
///   1. Namespace symbols: Erlang `-module(lists)`, Pascal `unit SysUtils`,
///      Clojure `(ns my.ns)` — explicit declarations.
///   2. File stems: `list.ml` → "list" and "List" (OCaml convention:
///      file modules are capitalized).
///
/// This enables precise module-qualified resolution: given `module="List"`
/// and `target_name="map"`, look up "List" → ["lib/list.ml"] → find "map"
/// among candidates in that file.
pub(super) fn build_module_to_files(parsed: &[ParsedFile]) -> FxHashMap<String, Vec<String>> {
    let mut map: FxHashMap<String, Vec<String>> = FxHashMap::default();

    for pf in parsed {
        if is_external_path(&pf.path) {
            continue;
        }
        // 1. Namespace symbols → module name (exact, authoritative)
        for sym in &pf.symbols {
            if sym.kind == SymbolKind::Namespace {
                let entry = map.entry(sym.name.clone()).or_default();
                if !entry.contains(&pf.path) {
                    entry.push(pf.path.clone());
                }
            }
        }

        // 2. File stem → module name
        let norm = pf.path.replace('\\', "/");
        if let Some(basename) = norm.rsplit('/').next() {
            if let Some(stem) = basename.rsplit_once('.').map(|(s, _)| s) {
                if !stem.is_empty() {
                    // Original case (e.g., "list" from "list.ml")
                    let entry = map.entry(stem.to_string()).or_default();
                    if !entry.contains(&pf.path) {
                        entry.push(pf.path.clone());
                    }
                    // Capitalized (OCaml/Haskell convention: file → Module)
                    let capitalized = capitalize_first(stem);
                    if capitalized != stem {
                        let entry = map.entry(capitalized).or_default();
                        if !entry.contains(&pf.path) {
                            entry.push(pf.path.clone());
                        }
                    }
                    // Lowercase for case-insensitive lookup
                    let lower = stem.to_lowercase();
                    if lower != stem {
                        let entry = map.entry(lower).or_default();
                        if !entry.contains(&pf.path) {
                            entry.push(pf.path.clone());
                        }
                    }
                }
            }
        }
    }
    map
}

/// Capitalize the first character of a string (for OCaml/Haskell module convention).
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

/// Build a per-file import map from the parsed extraction results.
///
/// Returns: file_path → Vec<(imported_name, module_path)>
///
/// For C#: `using FamilyBudget.Api.Entities;`
///         → ("FamilyBudget.Api.Entities", Some("FamilyBudget.Api.Entities"))  (EdgeKind::Imports)
/// For TS: `import { Foo } from "./foo"`
///         → ("Foo", Some("./foo"))  (EdgeKind::TypeRef with module)
pub(super) fn build_import_map(
    parsed: &[ParsedFile],
) -> FxHashMap<String, Vec<(String, Option<String>)>> {
    let mut map: FxHashMap<String, Vec<(String, Option<String>)>> = FxHashMap::default();
    for pf in parsed {
        if is_external_path(&pf.path) {
            continue;
        }
        for r in &pf.refs {
            match r.kind {
                EdgeKind::TypeRef if r.module.is_some() => {
                    // TypeScript-style: import { Foo } from "./bar"
                    map.entry(pf.path.clone())
                        .or_default()
                        .push((r.target_name.clone(), r.module.clone()));
                }
                EdgeKind::Imports => {
                    // C#-style: using FamilyBudget.Api.Entities
                    // The full namespace is stored in both target_name and module.
                    map.entry(pf.path.clone())
                        .or_default()
                        .push((r.target_name.clone(), r.module.clone()));
                }
                _ => {}
            }
        }
    }
    map
}

