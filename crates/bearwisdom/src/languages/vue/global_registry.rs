// =============================================================================
// languages/vue/global_registry.rs — Vue global component registration scanner
//
// Detects three forms of Vue global component registration, all of which cause
// PascalCase template refs to appear with no corresponding local import:
//
//   1. `app.component('FooBar', FooBar)`  — explicit single registration
//      (Vue 3 idiom; also handles `Vue.component('foo-bar', ...)` for Vue 2)
//
//   2. `app.use(LibraryPlugin)` / `Vue.use(LibraryPlugin)` — library plugin
//      registration that mass-registers every component in the library.
//      Only libraries with known stable name → component-name conventions are
//      detected:
//        element-ui  / element-plus  → El* prefix   (e.g. ElButton)
//        vuestic-ui                  → Va* prefix   (e.g. VaButton)
//        naive-ui                    → N*  prefix   (e.g. NButton)
//        ant-design-vue              → A*  prefix   (e.g. AButton)
//        @vben-core/shadcn-ui        → Vben* prefix (e.g. VbenButton)
//
//   3. `unplugin-vue-components` detected in `vite.config.ts` /
//      `vite.config.js` — signals that PascalCase components from configured
//      directories are auto-imported by the Vite/Webpack build plugin.  When
//      this plugin is active we record a special `UnpluginAutoImport` entry
//      that the resolver uses as a hint to try a by-name symbol lookup
//      rather than failing immediately.
//
// The scanner is **text-only** (no tree-sitter, no AST).  It reads the
// project's entry-point files and config files with simple regex heuristics.
// False-positive rate is low in practice: the scanner only touches a handful
// of files (main.ts/main.js, vite.config.*, package manifests), not the
// entire project tree.
//
// The output is a `VueGlobalRegistry` consumed by `VueResolver::build_file_context`
// to inject synthetic import entries for each registered component — so the
// existing TS import-based resolution chain can resolve them without special-casing
// the resolution engine.
// =============================================================================

use std::collections::HashMap;
use std::path::Path;
use tracing::debug;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The "source" of a globally-registered Vue component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VueComponentSource {
    /// The component was explicitly registered: `app.component('Name', Comp)`
    /// The value is the file path (relative to project root) where the
    /// registration call was found.
    ExplicitRegistration { file: String },

    /// All components from a named npm package (e.g. `element-plus`).
    /// The resolver injects `ImportEntry { module_path: Some(package) }` for
    /// every matching component name so the existing external-index lookup
    /// resolves it as `package.ComponentName`.
    Library { package: String },

    /// `unplugin-vue-components` is active — components are auto-discovered
    /// from configured directories.  When this is set the resolver falls back
    /// to a by-name symbol search for any PascalCase component that didn't
    /// resolve through imports.
    UnpluginAutoImport,
}

/// A project-wide Vue global component map.  Keys are PascalCase component
/// names; values describe where the component originates.
#[derive(Debug, Clone, Default)]
pub struct VueGlobalRegistry {
    pub components: HashMap<String, VueComponentSource>,
    /// True when `unplugin-vue-components` was detected — signals a blanket
    /// "any PascalCase component from the project tree may resolve by name".
    pub has_unplugin_auto_import: bool,
    /// When `has_unplugin_auto_import` is true, limit the by-name fallback to
    /// components whose names share a known package prefix.  Empty means "no
    /// prefix filter"; can be used to avoid spurious matches in projects that
    /// have both auto-import and many same-named internal symbols.
    pub auto_import_packages: Vec<String>,
}

impl VueGlobalRegistry {
    /// True when the registry has any information that can help the resolver.
    pub fn is_empty(&self) -> bool {
        self.components.is_empty() && !self.has_unplugin_auto_import
    }

    /// Return the library package name for a component name if any library
    /// covers it via its prefix convention.
    pub fn library_for(&self, component_name: &str) -> Option<&str> {
        match self.components.get(component_name) {
            Some(VueComponentSource::Library { package }) => Some(package.as_str()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Known library prefix conventions
// ---------------------------------------------------------------------------

/// Maps a (lowercased) npm package name to the PascalCase prefix its
/// auto-registered components use.  When a project uses `app.use(X)` and the
/// import says `import X from 'package'`, we classify all PascalCase refs
/// matching the prefix as coming from that package.
const LIBRARY_PREFIXES: &[(&str, &str)] = &[
    // Element UI (Vue 2)
    ("element-ui", "El"),
    // Element Plus (Vue 3)
    ("element-plus", "El"),
    // Vuestic UI
    ("vuestic-ui", "Va"),
    // Naive UI
    ("naive-ui", "N"),
    // Ant Design Vue
    ("ant-design-vue", "A"),
    // @vben-core/shadcn-ui (workspace package — detect by prefix)
    ("@vben-core/shadcn-ui", "Vben"),
    // Arco Design Vue
    ("@arco-design/web-vue", "A"),
    // Quasar (q- prefix)
    ("quasar", "Q"),
    // Vuetify (v- prefix normalizes to V)
    ("vuetify", "V"),
    // TDesign Vue
    ("tdesign-vue-next", "T"),
    ("tdesign-vue", "T"),
];

// ---------------------------------------------------------------------------
// Entry-point file candidates
// ---------------------------------------------------------------------------

/// File name stems considered project entry points.  The scanner tries these
/// paths (relative to project root and common sub-dirs) when looking for
/// `app.use` / `Vue.use` / `app.component` calls.
const ENTRY_STEMS: &[&str] = &["main.ts", "main.js", "bootstrap.ts", "bootstrap.js"];

/// Config file names where `unplugin-vue-components` may appear.
const VITE_CONFIG_NAMES: &[&str] = &["vite.config.ts", "vite.config.js", "vue.config.js"];

/// Directories to look in (relative to project root) when searching for entry
/// files.  Handles monorepos (apps/*/src/) with bounded depth.
const SEARCH_DIRS: &[&str] = &["", "src", "apps"];

// ---------------------------------------------------------------------------
// Public scanner entry point
// ---------------------------------------------------------------------------

/// Scan the project tree for Vue global component registrations and return a
/// `VueGlobalRegistry` describing what was found.
///
/// This is called once per full index, **before** the resolution pass.  The
/// returned registry is stored on `ProjectContext.vue_global_registry` and
/// consulted by `VueResolver::build_file_context`.
///
/// `parsed_paths` should be the list of relative paths of all project source
/// files, used to efficiently enumerate candidate entry-point files without
/// an additional filesystem walk.
pub fn scan_global_registrations(
    project_root: &Path,
    parsed_paths: &[String],
) -> VueGlobalRegistry {
    let mut registry = VueGlobalRegistry::default();

    // Check for unplugin-vue-components in vite.config.*
    let has_unplugin = detect_unplugin(project_root, parsed_paths);
    if has_unplugin {
        registry.has_unplugin_auto_import = true;
        debug!("vue: unplugin-vue-components detected");
    }

    // Scan entry-point files for app.use / Vue.use / app.component calls
    let entry_files = collect_entry_files(project_root, parsed_paths);
    debug!("vue: scanning {} entry files for global registrations", entry_files.len());

    for file_path in &entry_files {
        let rel_path = file_path
            .strip_prefix(project_root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_string();

        let source = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Extract `import X from 'pkg'` / `import { X } from 'pkg'` mappings
        let import_to_pkg = parse_imports(&source);

        // Detect app.use(X) / Vue.use(X) calls
        for (identifier, pkg_name) in detect_app_use_calls(&source, &import_to_pkg) {
            // Find which prefix this library uses
            let canonical_pkg = pkg_name.as_str();
            if let Some(prefix) = library_prefix_for(canonical_pkg) {
                // Register a sentinel for this prefix: when the resolver sees
                // a PascalCase name starting with `prefix`, look it up in
                // this package.
                //
                // We don't enumerate every possible component name here.
                // Instead, we register a special "_prefix_El" etc. entry and
                // let `library_for_name` do the prefix check at lookup time.
                let sentinel_key = format!("__prefix__{}", prefix);
                registry.components.insert(
                    sentinel_key,
                    VueComponentSource::Library {
                        package: canonical_pkg.to_string(),
                    },
                );
                debug!(
                    "vue: app.use({}) in {} → prefix '{}' → package '{}'",
                    identifier, rel_path, prefix, canonical_pkg
                );
            }
        }

        // Detect app.component('Name', Comp) / Vue.component('name', Comp) calls
        for component_name in detect_component_registrations(&source) {
            if !registry.components.contains_key(&component_name) {
                debug!(
                    "vue: explicit global component '{}' registered in {}",
                    component_name, rel_path
                );
                registry.components.insert(
                    component_name,
                    VueComponentSource::ExplicitRegistration {
                        file: rel_path.clone(),
                    },
                );
            }
        }
    }

    registry
}

/// Look up which package (if any) covers a given PascalCase component name by
/// checking the prefix sentinel entries in the registry.
///
/// Returns `Some(&package_name)` if a library with a matching prefix was detected.
pub fn library_for_name<'r>(registry: &'r VueGlobalRegistry, name: &str) -> Option<&'r str> {
    // First try exact component match
    if let Some(source) = registry.components.get(name) {
        if let VueComponentSource::Library { package } = source {
            return Some(package.as_str());
        }
    }
    // Then try prefix sentinels
    for (_, source) in &registry.components {
        if let VueComponentSource::Library { package } = source {
            if let Some(prefix) = library_prefix_for(package) {
                if name.starts_with(prefix) {
                    return Some(package.as_str());
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Unplugin detection
// ---------------------------------------------------------------------------

fn detect_unplugin(project_root: &Path, parsed_paths: &[String]) -> bool {
    for config_name in VITE_CONFIG_NAMES {
        // Check root level
        let root_path = project_root.join(config_name);
        if root_path.exists() {
            if file_contains_unplugin(&root_path) {
                return true;
            }
        }
        // Check parsed paths for config files in subdirs
        for rel_path in parsed_paths {
            let normalized = rel_path.replace('\\', "/");
            if normalized.ends_with(config_name) {
                let abs = project_root.join(rel_path.replace('\\', "/").trim_start_matches('/'));
                if abs.exists() && file_contains_unplugin(&abs) {
                    return true;
                }
            }
        }
    }
    false
}

fn file_contains_unplugin(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    content.contains("unplugin-vue-components")
}

// ---------------------------------------------------------------------------
// Entry file discovery
// ---------------------------------------------------------------------------

fn collect_entry_files(project_root: &Path, parsed_paths: &[String]) -> Vec<std::path::PathBuf> {
    let mut files: Vec<std::path::PathBuf> = Vec::new();

    // Strategy 1: look for known entry file names in parsed_paths (already
    // found by the walker, so existence is guaranteed).
    for rel_path in parsed_paths {
        let normalized = rel_path.replace('\\', "/");
        for stem in ENTRY_STEMS {
            if normalized.ends_with(stem) && !normalized.contains("node_modules") {
                let abs = project_root.join(rel_path.replace('\\', "/").trim_start_matches('/'));
                if !files.contains(&abs) {
                    files.push(abs);
                }
            }
        }
    }

    // Strategy 2: also look for vite.config.* files in case they register
    // global components directly (rare but possible).
    for config_name in VITE_CONFIG_NAMES {
        let root_path = project_root.join(config_name);
        if root_path.exists() && !files.contains(&root_path) {
            files.push(root_path);
        }
    }

    // Cap at 64 files to avoid O(N) scan on monorepos with hundreds of apps.
    // Sort by path depth (shallowest first) to prioritise root-level entries.
    files.sort_by_key(|p| p.components().count());
    files.truncate(64);
    files
}

// ---------------------------------------------------------------------------
// Import map extraction  (text-based, good enough for this scanner)
// ---------------------------------------------------------------------------

/// Parse `import X from 'pkg'` and `import { X, Y } from 'pkg'` into a map
/// from identifier → package name.
fn parse_imports(source: &str) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();

    for line in source.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("import") {
            continue;
        }
        // Match: import DefaultExport from 'pkg'
        // Match: import DefaultExport, { Named } from 'pkg'
        // Match: import * as Ns from 'pkg'
        // Match: import { Named, Other } from 'pkg'
        let pkg = extract_from_clause(trimmed);
        if pkg.is_empty() {
            continue;
        }
        // Extract identifier(s)
        if let Some(identifiers) = extract_import_identifiers(trimmed) {
            for id in identifiers {
                map.insert(id, pkg.clone());
            }
        }
    }
    map
}

/// Extract the package name from the `from 'pkg'` clause.
fn extract_from_clause(line: &str) -> String {
    // Find `from 'x'` or `from "x"`
    if let Some(from_idx) = line.rfind("from ") {
        let after = line[from_idx + 5..].trim();
        // Strip leading quote
        let quote = if after.starts_with('\'') {
            '\''
        } else if after.starts_with('"') {
            '"'
        } else if after.starts_with('`') {
            '`'
        } else {
            return String::new();
        };
        let rest = &after[1..];
        if let Some(end) = rest.find(quote) {
            return rest[..end].to_string();
        }
    }
    String::new()
}

/// Extract the bound identifier(s) from an import statement.
/// For `import Foo from 'x'` → `["Foo"]`
/// For `import { Bar, Baz } from 'x'` → `["Bar", "Baz"]`
/// For `import Foo, { Bar } from 'x'` → `["Foo", "Bar"]`
/// For `import * as Ns from 'x'` → `["Ns"]`
fn extract_import_identifiers(line: &str) -> Option<Vec<String>> {
    // Strip `import ` prefix
    let after_import = line.strip_prefix("import ")?.trim();

    // Remove the `from '...'` suffix (work backwards from `from `)
    let before_from = if let Some(idx) = after_import.rfind(" from ") {
        after_import[..idx].trim()
    } else {
        return None;
    };

    let mut ids: Vec<String> = Vec::new();

    // Handle `* as Ns`
    if before_from.starts_with("* as ") {
        let ns = before_from[5..].trim();
        if is_valid_ident(ns) {
            ids.push(ns.to_string());
        }
        return Some(ids);
    }

    // Handle `{ Bar, Baz as Z }` named imports — extract all identifiers
    // including renamed ones (`Foo as Bar` → we care about the local name `Bar`)
    let (default_part, named_part) = if let (Some(lo), Some(hi)) =
        (before_from.find('{'), before_from.rfind('}'))
    {
        let default_candidate = before_from[..lo].trim().trim_end_matches(',').trim();
        let named = &before_from[lo + 1..hi];
        (default_candidate, Some(named))
    } else {
        (before_from, None)
    };

    // Default / namespace import
    let default_part = default_part.trim_start_matches("type").trim();
    if !default_part.is_empty() && is_valid_ident(default_part) {
        ids.push(default_part.to_string());
    }

    // Named imports
    if let Some(named) = named_part {
        for item in named.split(',') {
            let item = item.trim();
            if item.is_empty() || item.starts_with("type ") {
                continue;
            }
            // `Foo as Bar` → local name is `Bar`
            let local = if let Some(as_idx) = item.find(" as ") {
                item[as_idx + 4..].trim()
            } else {
                item.trim()
            };
            if is_valid_ident(local) {
                ids.push(local.to_string());
            }
        }
    }

    if ids.is_empty() { None } else { Some(ids) }
}

fn is_valid_ident(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        && s.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_' || c == '$')
}

// ---------------------------------------------------------------------------
// app.use / Vue.use detection
// ---------------------------------------------------------------------------

/// Return `(identifier, package)` pairs for each `app.use(X)` / `Vue.use(X)`
/// call found in `source`, where the identifier was imported from a known
/// component library.
fn detect_app_use_calls(
    source: &str,
    import_to_pkg: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        // Match: app.use(X ...) or Vue.use(X ...) — allow whitespace
        let call = if let Some(rest) = trimmed.strip_prefix("app.use(") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("Vue.use(") {
            rest
        } else {
            continue;
        };

        // Extract first argument (identifier before `,` or `)`)
        let arg = call.split(|c| c == ',' || c == ')' || c == '(').next().unwrap_or("").trim();
        if arg.is_empty() {
            continue;
        }
        // Strip any wrapping `new ` prefix
        let arg = arg.strip_prefix("new ").unwrap_or(arg).trim();

        // Look up import source
        if let Some(pkg) = import_to_pkg.get(arg) {
            if is_known_component_library(pkg) {
                out.push((arg.to_string(), pkg.clone()));
            }
        }
    }
    out
}

fn is_known_component_library(pkg: &str) -> bool {
    LIBRARY_PREFIXES.iter().any(|(p, _)| *p == pkg)
}

fn library_prefix_for(pkg: &str) -> Option<&'static str> {
    LIBRARY_PREFIXES
        .iter()
        .find(|(p, _)| *p == pkg)
        .map(|(_, prefix)| *prefix)
}

// ---------------------------------------------------------------------------
// app.component / Vue.component detection
// ---------------------------------------------------------------------------

/// Return PascalCase component names from `app.component('name', ...)` /
/// `Vue.component('name', ...)` calls in `source`.
fn detect_component_registrations(source: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        // Match: app.component('name', ...) or Vue.component('name', ...)
        let call = if let Some(rest) = trimmed.strip_prefix("app.component(") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("Vue.component(") {
            rest
        } else {
            continue;
        };

        // Extract the string literal first argument
        let name_raw = call.trim();
        let name = extract_string_literal(name_raw);
        if name.is_empty() {
            continue;
        }

        // Normalize kebab-case → PascalCase.  Single-segment lowercase names
        // (e.g. 'mywidget') are capitalised so they satisfy the PascalCase
        // gate below and become 'Mywidget'.
        let pascal = kebab_to_pascal(&name);

        // Only collect if PascalCase (starts with uppercase)
        if pascal.chars().next().map_or(false, |c| c.is_uppercase()) {
            out.push(pascal);
        }
    }
    out
}

/// Extract the string literal at the start of `s` (strips quote, finds end
/// quote, handles both `'` and `"`).
fn extract_string_literal(s: &str) -> String {
    let quote = if s.starts_with('\'') {
        '\''
    } else if s.starts_with('"') {
        '"'
    } else {
        return String::new();
    };
    let rest = &s[1..];
    if let Some(end) = rest.find(quote) {
        rest[..end].to_string()
    } else {
        String::new()
    }
}

fn kebab_to_pascal(s: &str) -> String {
    s.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) fn _test_parse_imports(source: &str) -> HashMap<String, String> {
    parse_imports(source)
}

#[cfg(test)]
pub(crate) fn _test_detect_app_use(
    source: &str,
    import_map: &HashMap<String, String>,
) -> Vec<(String, String)> {
    detect_app_use_calls(source, import_map)
}

#[cfg(test)]
pub(crate) fn _test_detect_component_registrations(source: &str) -> Vec<String> {
    detect_component_registrations(source)
}

#[cfg(test)]
#[path = "global_registry_tests.rs"]
mod tests;
