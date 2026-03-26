// =============================================================================
// search/fuzzy.rs  —  Fuzzy file and symbol finder  (Phase 3)
//
// Ctrl+P / Ctrl+T equivalent.  Loads all file paths and symbol qualified names
// from the database into memory and runs nucleo-matcher against a pattern.
//
// Nucleo API used:
//   nucleo re-exports nucleo-matcher.  We use the Pattern::parse high-level
//   API for scoring and Atom::new + fuzzy_indices for extracting character
//   positions for highlighting.
// =============================================================================

use anyhow::{Context, Result};
use nucleo::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32String};
use serde::{Deserialize, Serialize};

use crate::db::Database;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzyMatch {
    /// The matched text (file path or qualified symbol name).
    pub text: String,
    /// Match score — higher is better.
    pub score: u32,
    /// Matched character positions within `text`, for highlight rendering.
    pub indices: Vec<u32>,
    pub metadata: FuzzyMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FuzzyMetadata {
    File { language: String },
    Symbol { kind: String, file_path: String, line: u32 },
}

// ---------------------------------------------------------------------------
// FuzzyIndex
// ---------------------------------------------------------------------------

/// In-memory snapshot of all indexable entries, ready for repeated matching.
///
/// Build once with `FuzzyIndex::from_db`, then call `match_files` /
/// `match_symbols` as often as needed without re-querying the DB.
pub struct FuzzyIndex {
    /// (path, language)
    file_entries: Vec<(String, String)>,
    /// (qualified_name, kind, file_path, line)
    symbol_entries: Vec<(String, String, String, u32)>,
}

impl FuzzyIndex {
    /// Load all file paths and symbol names from the database.
    pub fn from_db(db: &Database) -> Result<Self> {
        let conn = &db.conn;

        // Load files.
        let mut stmt = conn
            .prepare("SELECT path, language FROM files ORDER BY path")
            .context("Failed to prepare files query for FuzzyIndex")?;

        let file_entries: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .context("Failed to query files for FuzzyIndex")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect file entries")?;

        // Load symbols — join to get the file path.
        let mut stmt = conn
            .prepare(
                "SELECT s.qualified_name, s.kind, f.path, s.line
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 ORDER BY s.qualified_name",
            )
            .context("Failed to prepare symbols query for FuzzyIndex")?;

        let symbol_entries: Vec<(String, String, String, u32)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u32>(3)?,
                ))
            })
            .context("Failed to query symbols for FuzzyIndex")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect symbol entries")?;

        tracing::debug!(
            files = file_entries.len(),
            symbols = symbol_entries.len(),
            "FuzzyIndex loaded"
        );

        Ok(Self { file_entries, symbol_entries })
    }

    /// Fuzzy-match file paths against `pattern`.
    ///
    /// Returns up to `limit` results sorted by score descending.
    pub fn match_files(&self, pattern: &str, limit: usize) -> Vec<FuzzyMatch> {
        if pattern.is_empty() || limit == 0 {
            return Vec::new();
        }

        // Use path-aware config: bonus for path separators.
        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let pat =
            Pattern::parse(pattern, CaseMatching::Smart, Normalization::Smart);

        // match_list returns Vec<(&str, u16)> sorted by score descending.
        // We need the indices too, so we score first then re-run for indices.
        let candidates: Vec<&str> =
            self.file_entries.iter().map(|(p, _)| p.as_str()).collect();

        let scored = pat.match_list(candidates, &mut matcher);

        let mut results: Vec<FuzzyMatch> = scored
            .into_iter()
            .take(limit)
            .filter_map(|(path, score)| {
                // Find the entry to get the language.
                let language = self
                    .file_entries
                    .iter()
                    .find(|(p, _)| p == path)
                    .map(|(_, l)| l.clone())
                    .unwrap_or_default();

                let indices = extract_indices(path, pattern, false);

                Some(FuzzyMatch {
                    text: path.to_string(),
                    score: score as u32,
                    indices,
                    metadata: FuzzyMetadata::File { language },
                })
            })
            .collect();

        results.sort_by(|a, b| b.score.cmp(&a.score));
        results
    }

    /// Fuzzy-match symbol qualified names against `pattern`.
    ///
    /// Returns up to `limit` results sorted by score descending.
    pub fn match_symbols(&self, pattern: &str, limit: usize) -> Vec<FuzzyMatch> {
        if pattern.is_empty() || limit == 0 {
            return Vec::new();
        }

        let mut matcher = Matcher::new(Config::DEFAULT);
        let pat =
            Pattern::parse(pattern, CaseMatching::Smart, Normalization::Smart);

        let candidates: Vec<&str> =
            self.symbol_entries.iter().map(|(n, _, _, _)| n.as_str()).collect();

        let scored = pat.match_list(candidates, &mut matcher);

        let mut results: Vec<FuzzyMatch> = scored
            .into_iter()
            .take(limit)
            .filter_map(|(qname, score)| {
                let entry = self
                    .symbol_entries
                    .iter()
                    .find(|(n, _, _, _)| n == qname)?;

                let (_, kind, file_path, line) = entry;
                let indices = extract_indices(qname, pattern, false);

                Some(FuzzyMatch {
                    text: qname.to_string(),
                    score: score as u32,
                    indices,
                    metadata: FuzzyMetadata::Symbol {
                        kind: kind.clone(),
                        file_path: file_path.clone(),
                        line: *line,
                    },
                })
            })
            .collect();

        results.sort_by(|a, b| b.score.cmp(&a.score));
        results
    }

    /// Number of file entries loaded.
    pub fn file_count(&self) -> usize {
        self.file_entries.len()
    }

    /// Number of symbol entries loaded.
    pub fn symbol_count(&self) -> usize {
        self.symbol_entries.len()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract matched character indices for a haystack/pattern pair.
///
/// Uses nucleo's `Atom::fuzzy_indices` under the hood.  Falls back to an
/// empty vec on any failure (indices are optional — they are only used for
/// highlight rendering).
fn extract_indices(haystack: &str, pattern: &str, match_paths: bool) -> Vec<u32> {
    use nucleo::pattern::Atom;

    let config = if match_paths {
        Config::DEFAULT.match_paths()
    } else {
        Config::DEFAULT
    };
    let mut matcher = Matcher::new(config);

    // Build the haystack as a Utf32String so nucleo can work with it.
    let haystack_u32: Utf32String = haystack.into();

    let atom = Atom::new(
        pattern,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
        false,
    );

    let mut indices: Vec<u32> = Vec::new();
    atom.indices(haystack_u32.slice(..), &mut matcher, &mut indices);
    indices.sort_unstable();
    indices.dedup();
    indices
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "fuzzy_tests.rs"]
mod tests;
