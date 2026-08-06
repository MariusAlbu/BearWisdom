// =============================================================================
// ecosystem/symbol_index.rs — cheap (module, name) → file lookup for externals
//
// An ecosystem's `build_symbol_index` walks every reached dep root and, using
// a header-only tree-sitter parse (top-level declarations only, no function
// body descent), registers each top-level decl's name against the file that
// defines it. Stage 2 of the refactored pipeline consults this index in
// Phase A of its loop: for every (module, name) in the current demand set,
// `locate` returns the exact file to pull and parse.
//
// Shape choice: owned `(module, name) → PathBuf` rather than
// `Arc<ExternalDepRoot>` references so callers can freely move results
// across threads without lifetime juggling. Index construction is the
// expensive half; lookups are one HashMap probe.
//
// Header-only-parse rationale (from the design discussion):
//   * Regex scans miss multi-line signatures, build-tag-gated decls, and
//     oddly formatted sources — accuracy matters because a miss here means
//     the chain walker records a spurious demand that never resolves.
//   * Tree-sitter parsing is already the crate's extraction substrate. We
//     reuse grammars instead of introducing a second parser family.
//   * Skipping function bodies is what makes this cheap: bodies dominate
//     AST size and we don't need them to know a decl's name.
//
// This file holds the data shape + the query surface. The per-ecosystem
// scanner implementation lands alongside each ecosystem's migration to
// demand-driven external parsing.
// =============================================================================

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A lookup handle mapping `(module_path, symbol_name)` to the absolute path
/// of the file that defines the symbol at top level.
///
/// `module_path` is the ecosystem-native package identifier (npm package
/// name, Go module path, PyPI distribution name, Maven `group:artifact`,
/// etc.). `symbol_name` is the short name of a top-level declaration —
/// `Open`, `DB.Query`, `useState`, `FastAPI`, as reported by the header-only
/// scan. Methods are keyed `ReceiverType.MethodName` so chain walkers can
/// locate them from the receiver's type.
///
/// A secondary `name → Vec<(module, file)>` index backs `find_by_name` so
/// the chain-expansion hot path doesn't scan every entry per miss — that
/// was the O(N × refs) cost that made demand-driven Go 2× slower than the
/// eager path on pocketbase.
///
/// Empty indexes are valid — they mean "no external demand can be answered
/// for this ecosystem, fall back to eager walk."
#[derive(Debug, Default, Clone)]
pub struct SymbolLocationIndex {
    entries: HashMap<(String, String), PathBuf>,
    by_name: HashMap<String, Vec<(String, PathBuf)>>,
    /// `module_path → the package's `.` entry file`. A barrel package re-exports
    /// names defined in OTHER packages, so the leaf is keyed under the defining
    /// package in `entries`, not the imported one. Materializing the entry brings
    /// in the package's `export *` chain so re-export-following can bind the import.
    module_entries: HashMap<String, PathBuf>,
    /// Cross-package re-export bridges: `(module, name)` is bound by `module`'s
    /// public surface, but the declaration lives in ANOTHER package's file. The
    /// `(module, name)` slot in `entries` keeps pointing at the re-exporting
    /// barrel (materialized symbols are qname-prefixed by their own file's
    /// package, so relocating the slot would key the wrong prefix); this map
    /// carries the declaration's file + declared name so the lookup layer can
    /// register `{module}.{name}` as a qname ALIAS of that single declaration.
    reexport_aliases: HashMap<(String, String), (PathBuf, String)>,
}

impl SymbolLocationIndex {
    /// Construct an empty index. Ecosystems that have not yet migrated to
    /// demand-driven parsing return this from the default trait impl.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `symbol_name` exported by `module_path` is defined in
    /// `file`. First writer wins on the `(module, name)` axis. The
    /// `name → files` reverse index accumulates every (module, file) that
    /// declares the name, so `find_by_name` can return all matches without
    /// re-scanning.
    pub fn insert(
        &mut self,
        module_path: impl Into<String>,
        symbol_name: impl Into<String>,
        file: impl Into<PathBuf>,
    ) {
        let module = module_path.into();
        let name = symbol_name.into();
        let file = file.into();
        self.entries
            .entry((module.clone(), name.clone()))
            .or_insert_with(|| file.clone());
        self.by_name.entry(name).or_default().push((module, file));
    }

    /// Return the file that defines `symbol_name` inside `module_path`,
    /// or `None` when the name is unknown to this ecosystem.
    pub fn locate(&self, module_path: &str, symbol_name: &str) -> Option<&Path> {
        self.entries
            .get(&(module_path.to_string(), symbol_name.to_string()))
            .map(PathBuf::as_path)
    }

    /// Record a package's `.` entry file (first writer wins).
    pub fn insert_module_entry(
        &mut self,
        module_path: impl Into<String>,
        file: impl Into<PathBuf>,
    ) {
        self.module_entries
            .entry(module_path.into())
            .or_insert_with(|| file.into());
    }

    /// The `.` entry file of `module_path`, when known. Pulled by the demand pass
    /// for a module-tagged ref so a barrel package's re-export chain materializes.
    pub fn module_entry(&self, module_path: &str) -> Option<&Path> {
        self.module_entries.get(module_path).map(PathBuf::as_path)
    }

    /// Record a cross-package re-export bridge: `module` binds `name`, whose
    /// declaration is `target_name` in `target_file` (another package). Skipped
    /// when `(module, name)` already has a located definition — a real local
    /// declaration owns the slot, and an alias must never shadow it. First
    /// writer wins on the alias axis, matching `insert`.
    pub fn push_reexport_alias(
        &mut self,
        module: impl Into<String>,
        name: impl Into<String>,
        target_file: impl Into<PathBuf>,
        target_name: impl Into<String>,
    ) {
        let key = (module.into(), name.into());
        if self.entries.contains_key(&key) {
            return;
        }
        self.reexport_aliases
            .entry(key)
            .or_insert_with(|| (target_file.into(), target_name.into()));
    }

    /// Every recorded cross-package bridge as
    /// `(module, name, target_file, target_name)`.
    pub fn reexport_aliases(&self) -> impl Iterator<Item = (&str, &str, &Path, &str)> {
        self.reexport_aliases.iter().map(|((module, name), (file, target_name))| {
            (module.as_str(), name.as_str(), file.as_path(), target_name.as_str())
        })
    }

    /// Return every `(module_path, file)` pair where the symbol's short
    /// name matches `symbol_name`. Used by the demand-driven pipeline to
    /// resolve chain-walker bail-outs: the walker only knows "I was
    /// looking for DB.Query", not which module DB lives in, so the index
    /// has to sweep modules for a match.
    ///
    /// O(1) lookup + O(k) copy where k is the number of modules whose
    /// top-level declares this name (usually 0 or 1; occasionally a
    /// handful for common names like `Client` that appear across packages).
    pub fn find_by_name(&self, symbol_name: &str) -> Vec<(&str, &Path)> {
        self.by_name
            .get(symbol_name)
            .map(|v| v.iter().map(|(m, p)| (m.as_str(), p.as_path())).collect())
            .unwrap_or_default()
    }

    /// Merge another index into this one. Existing entries are preserved
    /// on the `(module, name)` axis (first-writer-wins); the reverse
    /// `name → files` index accumulates every incoming entry so merges
    /// don't lose cross-module locations.
    pub fn extend(&mut self, other: SymbolLocationIndex) {
        for ((module, name), file) in other.entries {
            self.entries.entry((module, name)).or_insert(file);
        }
        // Merge the reverse index DIRECTLY — re-deriving it from `entries`
        // (a first-wins map) would drop every same-(module, name) sibling
        // beyond the first: two static classes in one package offering the
        // same method name must BOTH stay locatable by name.
        for (name, locs) in other.by_name {
            self.by_name.entry(name).or_default().extend(locs);
        }
        for (module, entry) in other.module_entries {
            self.module_entries.entry(module).or_insert(entry);
        }
        // Aliases coexist with their own barrel-fallback entry (merged above),
        // so no entries-vacancy check here — `push_reexport_alias` enforced it
        // against genuine local declarations at build time.
        for (key, target) in other.reexport_aliases {
            self.reexport_aliases.entry(key).or_insert(target);
        }
    }

    /// Number of recorded (module, name) pairs — diagnostic only.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
#[path = "symbol_index_tests.rs"]
mod tests;
