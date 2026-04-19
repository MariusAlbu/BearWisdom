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
}

impl SymbolLocationIndex {
    /// Construct an empty index. Ecosystems that have not yet migrated to
    /// demand-driven parsing return this from the default trait impl.
    pub fn new() -> Self { Self::default() }

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
        self.by_name
            .entry(name)
            .or_default()
            .push((module, file));
    }

    /// Return the file that defines `symbol_name` inside `module_path`,
    /// or `None` when the name is unknown to this ecosystem.
    pub fn locate(&self, module_path: &str, symbol_name: &str) -> Option<&Path> {
        self.entries
            .get(&(module_path.to_string(), symbol_name.to_string()))
            .map(PathBuf::as_path)
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
            .map(|v| {
                v.iter()
                    .map(|(m, p)| (m.as_str(), p.as_path()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Merge another index into this one. Existing entries are preserved
    /// on the `(module, name)` axis (first-writer-wins); the reverse
    /// `name → files` index accumulates every incoming entry so merges
    /// don't lose cross-module locations.
    pub fn extend(&mut self, other: SymbolLocationIndex) {
        for ((module, name), file) in other.entries {
            self.entries
                .entry((module.clone(), name.clone()))
                .or_insert_with(|| file.clone());
            self.by_name
                .entry(name)
                .or_default()
                .push((module, file));
        }
    }

    /// Number of recorded (module, name) pairs — diagnostic only.
    pub fn len(&self) -> usize { self.entries.len() }

    /// Whether the index has no entries.
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_locate_roundtrip() {
        let mut idx = SymbolLocationIndex::new();
        idx.insert("modernc.org/sqlite", "Open", "/cache/sqlite/sqlite.go");
        let hit = idx.locate("modernc.org/sqlite", "Open");
        assert_eq!(hit, Some(Path::new("/cache/sqlite/sqlite.go")));
    }

    #[test]
    fn miss_returns_none() {
        let idx = SymbolLocationIndex::new();
        assert!(idx.locate("anything", "anything").is_none());
    }

    #[test]
    fn first_writer_wins_on_duplicate() {
        let mut idx = SymbolLocationIndex::new();
        idx.insert("pkg", "Foo", "/a.go");
        idx.insert("pkg", "Foo", "/b.go");
        assert_eq!(idx.locate("pkg", "Foo"), Some(Path::new("/a.go")));
    }

    #[test]
    fn extend_preserves_existing_entries() {
        let mut base = SymbolLocationIndex::new();
        base.insert("pkg", "Foo", "/a.go");
        let mut other = SymbolLocationIndex::new();
        other.insert("pkg", "Foo", "/b.go");
        other.insert("pkg", "Bar", "/c.go");
        base.extend(other);
        assert_eq!(base.locate("pkg", "Foo"), Some(Path::new("/a.go")));
        assert_eq!(base.locate("pkg", "Bar"), Some(Path::new("/c.go")));
        assert_eq!(base.len(), 2);
    }

    #[test]
    fn empty_by_default() {
        let idx = SymbolLocationIndex::new();
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn find_by_name_returns_all_modules_with_match() {
        let mut idx = SymbolLocationIndex::new();
        idx.insert("pkg-a", "Query", "/a/query.rs");
        idx.insert("pkg-b", "Query", "/b/query.rs");
        idx.insert("pkg-a", "Other", "/a/other.rs");

        let mut hits = idx.find_by_name("Query");
        hits.sort_by_key(|(m, _)| *m);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, "pkg-a");
        assert_eq!(hits[0].1, Path::new("/a/query.rs"));
        assert_eq!(hits[1].0, "pkg-b");
        assert_eq!(hits[1].1, Path::new("/b/query.rs"));
    }

    #[test]
    fn find_by_name_empty_when_no_match() {
        let mut idx = SymbolLocationIndex::new();
        idx.insert("pkg", "Foo", "/x.rs");
        assert!(idx.find_by_name("NotThere").is_empty());
    }
}
