// =============================================================================
// query/completion.rs  —  scope-aware auto-completion
//
// Given a cursor position (file, line, col) and a prefix string, returns
// ranked completion candidates from three tiers:
//
//   Tier 0 — Same scope: siblings within the containing symbol's scope_path.
//   Tier 1 — Imported: symbols reachable via the file's import/using directives.
//   Tier 2 — Namespace peers: symbols sharing the same top-level namespace.
//
// Candidates are fuzzy-filtered by `prefix`, then ranked by scope distance,
// match score, and call frequency (incoming edge count).
// =============================================================================

use crate::db::Database;
use anyhow::{Context, Result};
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32String};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

/// A single completion candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionItem {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_summary: Option<String>,
    /// 0 = same scope, 1 = imported, 2 = namespace peer.
    pub scope_distance: u32,
    /// Nucleo fuzzy match score (higher = better).
    pub score: u32,
}

// ---------------------------------------------------------------------------
// Public function
// ---------------------------------------------------------------------------

/// Return completion candidates for a cursor position.
///
/// `prefix` is the partial text typed so far (may be empty for "show all").
/// Returns at most 30 candidates, ranked by scope distance then match score.
pub fn complete_at(
    db: &Database,
    file_path: &str,
    line: u32,
    _col: u32,
    prefix: &str,
    include_signature: bool,
) -> Result<Vec<CompletionItem>> {
    let _timer = db.timer("complete_at");
    let conn = &db.conn;

    // --- Step 1: Resolve file_id and containing scope ---
    let file_id: Option<i64> = conn
        .query_row("SELECT id FROM files WHERE path = ?1", [file_path], |r| r.get(0))
        .optional()
        .context("completion: file lookup")?;

    let Some(file_id) = file_id else {
        return Ok(vec![]);
    };

    // Find the narrowest symbol containing the cursor line.
    let containing_scope: Option<String> = conn
        .query_row(
            "SELECT qualified_name FROM symbols
             WHERE file_id = ?1 AND line <= ?2 AND COALESCE(end_line, line) >= ?2
             ORDER BY (COALESCE(end_line, line) - line) ASC
             LIMIT 1",
            rusqlite::params![file_id, line],
            |r| r.get(0),
        )
        .optional()
        .context("completion: scope lookup")?;

    let sig_col = if include_signature { "s.signature" } else { "NULL" };

    // --- Step 2: Collect candidates from three tiers ---
    let mut candidates: Vec<(CompletionItem, u32)> = Vec::new(); // (item, raw_score for dedup)

    // Tier 0: same-scope siblings
    if let Some(ref scope) = containing_scope {
        let sql = format!(
            "SELECT s.name, s.qualified_name, s.kind, f.path, {sig_col},
                    SUBSTR(s.doc_comment, 1, 80)
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE s.scope_path = ?1
             LIMIT 200"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([scope], |row| {
            Ok(CompletionItem {
                name: row.get(0)?,
                qualified_name: row.get(1)?,
                kind: row.get(2)?,
                file_path: row.get(3)?,
                signature: row.get(4)?,
                doc_summary: row.get(5)?,
                scope_distance: 0,
                score: 0,
            })
        })?;
        for row in rows.flatten() {
            candidates.push((row, 0));
        }
    }

    // Tier 1: imported symbols
    {
        let sql = format!(
            "SELECT s.name, s.qualified_name, s.kind, f.path, {sig_col},
                    SUBSTR(s.doc_comment, 1, 80)
             FROM imports i
             JOIN symbols s ON s.name = i.imported_name
             JOIN files f ON f.id = s.file_id
             WHERE i.file_id = ?1
             LIMIT 200"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([file_id], |row| {
            Ok(CompletionItem {
                name: row.get(0)?,
                qualified_name: row.get(1)?,
                kind: row.get(2)?,
                file_path: row.get(3)?,
                signature: row.get(4)?,
                doc_summary: row.get(5)?,
                scope_distance: 1,
                score: 0,
            })
        })?;
        for row in rows.flatten() {
            candidates.push((row, 0));
        }
    }

    // Tier 2: namespace peers (same top-level namespace as containing scope)
    if let Some(ref scope) = containing_scope {
        // Extract namespace: take everything up to the last dot.
        if let Some(dot_pos) = scope.rfind('.') {
            let namespace = &scope[..dot_pos];
            let like_pattern = format!("{namespace}.%");
            let sql = format!(
                "SELECT s.name, s.qualified_name, s.kind, f.path, {sig_col},
                        SUBSTR(s.doc_comment, 1, 80)
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE s.scope_path LIKE ?1
                   AND s.scope_path != ?2
                 LIMIT 200"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params![like_pattern, scope], |row| {
                Ok(CompletionItem {
                    name: row.get(0)?,
                    qualified_name: row.get(1)?,
                    kind: row.get(2)?,
                    file_path: row.get(3)?,
                    signature: row.get(4)?,
                    doc_summary: row.get(5)?,
                    scope_distance: 2,
                    score: 0,
                })
            })?;
            for row in rows.flatten() {
                candidates.push((row, 0));
            }
        }
    }

    // --- Step 3: Fuzzy filter by prefix ---
    if !prefix.is_empty() {
        let mut matcher = Matcher::new(Config::DEFAULT);
        let pat = Pattern::parse(prefix, CaseMatching::Smart, Normalization::Smart);

        // Score each candidate.
        candidates.retain_mut(|(item, _)| {
            let haystack = Utf32String::from(item.name.as_str());
            match pat.score(haystack.slice(..), &mut matcher) {
                Some(score) => {
                    item.score = score;
                    true
                }
                None => false,
            }
        });
    } else {
        // No prefix — keep all, assign score 1 so sorting works.
        for (item, _) in &mut candidates {
            item.score = 1;
        }
    }

    // Dedup by qualified_name (keep lowest scope_distance).
    {
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|(item, _)| seen.insert(item.qualified_name.clone()));
    }

    // --- Step 4: Rank ---
    // Primary: scope_distance ASC, secondary: score DESC.
    candidates.sort_by(|(a, _), (b, _)| {
        a.scope_distance
            .cmp(&b.scope_distance)
            .then_with(|| b.score.cmp(&a.score))
    });

    // Return top 30.
    let results: Vec<CompletionItem> = candidates
        .into_iter()
        .take(30)
        .map(|(item, _)| item)
        .collect();

    Ok(results)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_complete_at_empty_db() {
        let db = Database::open_in_memory().unwrap();
        let results = complete_at(&db, "nonexistent.rs", 1, 0, "foo", false).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_complete_at_same_scope() {
        let db = Database::open_in_memory().unwrap();
        db.conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/svc.rs', 'h', 'rust', 0)",
            [],
        ).unwrap();
        let file_id = db.conn.last_insert_rowid();

        // Parent class
        db.conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, end_line, scope_path)
             VALUES (?1, 'MyService', 'app.MyService', 'class', 1, 0, 50, 'app')",
            [file_id],
        ).unwrap();

        // Methods (scope_path = qualified_name of parent)
        db.conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, scope_path)
             VALUES (?1, 'get_item', 'app.MyService.get_item', 'method', 5, 0, 'app.MyService')",
            [file_id],
        ).unwrap();
        db.conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, scope_path)
             VALUES (?1, 'get_all', 'app.MyService.get_all', 'method', 15, 0, 'app.MyService')",
            [file_id],
        ).unwrap();
        db.conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, scope_path)
             VALUES (?1, 'delete', 'app.MyService.delete', 'method', 25, 0, 'app.MyService')",
            [file_id],
        ).unwrap();

        // Complete at line 10 (inside MyService), prefix "get"
        let results = complete_at(&db, "src/svc.rs", 10, 0, "get", false).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.name.starts_with("get")));
        assert!(results.iter().all(|r| r.scope_distance == 0));
    }

    #[test]
    fn test_complete_at_no_prefix_returns_all() {
        let db = Database::open_in_memory().unwrap();
        db.conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/a.rs', 'h', 'rust', 0)",
            [],
        ).unwrap();
        let fid = db.conn.last_insert_rowid();

        db.conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, end_line, scope_path)
             VALUES (?1, 'Outer', 'mod.Outer', 'class', 1, 0, 30, 'mod')",
            [fid],
        ).unwrap();
        db.conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, scope_path)
             VALUES (?1, 'alpha', 'mod.Outer.alpha', 'method', 3, 0, 'mod.Outer')",
            [fid],
        ).unwrap();
        db.conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, scope_path)
             VALUES (?1, 'beta', 'mod.Outer.beta', 'method', 10, 0, 'mod.Outer')",
            [fid],
        ).unwrap();

        let results = complete_at(&db, "src/a.rs", 5, 0, "", false).unwrap();
        assert!(results.len() >= 2);
    }
}
