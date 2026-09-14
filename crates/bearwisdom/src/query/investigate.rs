// =============================================================================
// query/investigate.rs  —  composite "deep-dive" query
//
// Combines symbol_info + incoming_calls + outgoing_calls + blast_radius
// into a single round-trip.  Designed for LLM consumption — one tool call
// instead of four.
// =============================================================================

use crate::db::Database;
use crate::query::blast_radius::{self, AffectedSymbol};
use crate::query::call_hierarchy::{self, CallHierarchyItem};
use crate::query::references::{self, ReferenceCoverage, SourceAttestedOccurrence};
use crate::query::QueryResult;
use crate::types::ReferenceResult;
use anyhow::Context;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Controls the scope of an investigate query.
#[derive(Debug, Clone)]
pub struct InvestigateOptions {
    /// Maximum number of resolved reference occurrences to return.
    pub reference_limit: usize,
    /// Maximum number of callers to return.
    pub caller_limit: usize,
    /// Maximum number of callees to return.
    pub callee_limit: usize,
    /// Blast radius traversal depth (1 = direct dependents only).
    pub blast_depth: u32,
}

impl Default for InvestigateOptions {
    fn default() -> Self {
        Self {
            reference_limit: 20,
            caller_limit: 10,
            callee_limit: 10,
            blast_depth: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Slim symbol summary used as the center of an investigate result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlimSymbol {
    /// Stable index identity when this snapshot was written by the
    /// symbol-identity pipeline.  Missing on legacy rows; consumers must not
    /// substitute a qualified-name string for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_id: Option<String>,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file_path: String,
    pub line: u32,
    pub signature: Option<String>,
}

/// Bounded indexed source surrounding the selected declaration.  Code chunks
/// are content-hash keyed, so this excerpt belongs to the same snapshot as the
/// symbol and does not require a separate filesystem read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceExcerpt {
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
    pub truncated: bool,
}

/// A test attached by a resolved reference or by a unique-name source call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearbyTest {
    pub name: String,
    pub qualified_name: String,
    pub file_path: String,
    pub line: u32,
}

/// Combined result of an investigate query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigateResult {
    /// The symbol being investigated.
    pub symbol: SlimSymbol,
    /// Bounded declaration body from the indexed code-chunk snapshot.
    pub source_excerpt: Option<SourceExcerpt>,
    /// Tests in the same file or directly exercising this declaration.
    pub nearby_tests: Vec<NearbyTest>,
    /// Resolved occurrences that point at the selected symbol identity.
    pub references: Vec<ReferenceResult>,
    /// Source-attested unresolved or drained occurrences whose emitted name
    /// matches the selected declaration's simple name. These are deliberately
    /// separate from `references`: name-only evidence never proves a graph
    /// relationship, especially when `candidate_declaration_count` is > 1.
    pub source_attested_unresolved: Vec<SourceAttestedOccurrence>,
    /// Counts and source provenance for both occurrence lists. An empty
    /// `references` vector is only an empty resolved set; consult this field
    /// before drawing any conclusion about coverage or source occurrences.
    pub reference_coverage: ReferenceCoverage,
    /// Symbols that call this symbol (incoming call hierarchy).
    pub callers: Vec<CallHierarchyItem>,
    /// Symbols that this symbol calls (outgoing call hierarchy).
    pub callees: Vec<CallHierarchyItem>,
    /// Blast radius — what breaks if this symbol changes.
    pub blast_radius: Option<BlastRadiusSlim>,
}

/// Slim blast radius — just the count and affected list, no center repeat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlastRadiusSlim {
    pub total_affected: u32,
    pub affected: Vec<AffectedSymbol>,
}

// ---------------------------------------------------------------------------
// Public function
// ---------------------------------------------------------------------------

/// Deep-dive into a symbol: returns identity, references, callers, callees,
/// and blast radius.
///
/// `symbol_name` may be a simple name or fully-qualified name.
/// Returns `Ok(None)` if the symbol is not found.
pub fn investigate(
    db: &Database,
    symbol_name: &str,
    opts: &InvestigateOptions,
) -> QueryResult<Option<InvestigateResult>> {
    let _timer = db.timer("investigate");
    let conn = db.conn();

    // --- Resolve the symbol ---
    let lookup_sql =
        "SELECT s.id, s.symbol_key, s.name, s.qualified_name, s.kind, f.path, s.line, s.signature
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE (s.qualified_name = ?1 OR s.name = ?1)
           AND s.origin = 'internal'
         ORDER BY CASE WHEN s.qualified_name = ?1 AND s.name <> ?1 THEN 0 ELSE 1 END,
                  s.qualified_name
         LIMIT 1";

    let row = conn
        .query_row(lookup_sql, [symbol_name], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, u32>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .optional()
        .context("investigate: symbol lookup")?;

    let Some((id, symbol_id, name, qualified_name, kind, file_path, line, signature)) = row else {
        return Ok(None);
    };

    let symbol = SlimSymbol {
        symbol_id,
        name: name.clone(),
        qualified_name: qualified_name.clone(),
        kind,
        file_path,
        line,
        signature,
    };
    let source_excerpt = source_excerpt(conn, id, &symbol.file_path, symbol.line)?;
    let nearby_tests = nearby_tests(conn, id, &name, 8)?;

    // --- Resolved graph evidence ---
    // Query failures must propagate: turning a busy/corrupt graph into an
    // empty section would make the evidence look complete when it is not.
    let reference_evidence =
        references::evidence_for_declaration(db, id, &name, opts.reference_limit)?;

    // Use the selected qualified identity for all graph queries so namesakes
    // and overload groups do not leak into the evidence bundle.
    let callers = call_hierarchy::incoming_calls(db, &qualified_name, opts.caller_limit)?;

    // --- Callees ---
    let callees = call_hierarchy::outgoing_calls(db, &qualified_name, opts.callee_limit)?;

    // --- Blast radius ---
    let blast_radius =
        blast_radius::blast_radius(db, &qualified_name, opts.blast_depth, 500)?.map(|br| {
            BlastRadiusSlim {
                total_affected: br.total_affected,
                affected: br.affected,
            }
        });

    Ok(Some(InvestigateResult {
        symbol,
        source_excerpt,
        nearby_tests,
        references: reference_evidence.resolved,
        source_attested_unresolved: reference_evidence.source_attested_unresolved,
        reference_coverage: reference_evidence.coverage,
        callers,
        callees,
        blast_radius,
    }))
}

fn source_excerpt(
    conn: &rusqlite::Connection,
    symbol_id: i64,
    file_path: &str,
    symbol_line: u32,
) -> QueryResult<Option<SourceExcerpt>> {
    const MAX_EXCERPT_CHARS: usize = 2400;
    let row = conn
        .query_row(
            "SELECT c.content, c.start_line, c.end_line
             FROM code_chunks c
             JOIN symbols s ON s.id = ?1 AND s.file_id = c.file_id
             WHERE c.symbol_id = ?1 OR (c.start_line <= ?2 AND c.end_line >= ?2)
             ORDER BY CASE WHEN c.symbol_id = ?1 THEN 0 ELSE 1 END,
                      (c.end_line - c.start_line)
             LIMIT 1",
            rusqlite::params![symbol_id, symbol_line],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, u32>(2)?,
                ))
            },
        )
        .optional()
        .context("investigate: source excerpt")?;
    let Some((content, start_line, end_line)) = row else {
        return Ok(None);
    };
    let truncated = content.chars().count() > MAX_EXCERPT_CHARS;
    let content = if truncated {
        content.chars().take(MAX_EXCERPT_CHARS).collect()
    } else {
        content
    };
    Ok(Some(SourceExcerpt {
        file_path: file_path.to_owned(),
        start_line,
        end_line,
        content,
        truncated,
    }))
}

fn nearby_tests(
    conn: &rusqlite::Connection,
    symbol_id: i64,
    symbol_name: &str,
    limit: usize,
) -> QueryResult<Vec<NearbyTest>> {
    let mut stmt = conn
        .prepare(
            "WITH target AS (SELECT file_id FROM symbols WHERE id = ?1),
                  unique_name AS (
                    SELECT COUNT(*) = 1 AS is_unique
                    FROM symbols WHERE name = ?2 AND origin = 'internal'
                  ),
                  candidates AS (
                    SELECT s.name, s.qualified_name, f.path, s.line
                    FROM symbols s JOIN files f ON f.id = s.file_id, target
                    WHERE s.kind = 'test' AND s.file_id = target.file_id
                    UNION
                    SELECT s.name, s.qualified_name, f.path, s.line
                    FROM ref_resolutions rr
                    JOIN symbols s ON s.id = rr.source_id
                    JOIN files f ON f.id = s.file_id
                    WHERE s.kind = 'test'
                      AND (rr.target_id = ?1 OR (
                        rr.target_name = ?2 AND (SELECT is_unique FROM unique_name)
                      ))
                  )
             SELECT name, qualified_name, path, line
             FROM candidates ORDER BY path, line LIMIT ?3",
        )
        .context("investigate: prepare nearby tests")?;
    let rows = stmt
        .query_map(
            rusqlite::params![symbol_id, symbol_name, limit as i64],
            |row| {
                Ok(NearbyTest {
                    name: row.get(0)?,
                    qualified_name: row.get(1)?,
                    file_path: row.get(2)?,
                    line: row.get(3)?,
                })
            },
        )
        .context("investigate: query nearby tests")?;
    Ok(rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("investigate: collect nearby tests")?)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_investigate_not_found() {
        let db = Database::open_in_memory().unwrap();
        let result = investigate(&db, "nonexistent", &InvestigateOptions::default()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_investigate_found() {
        let db = Database::open_in_memory().unwrap();

        db.conn().execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/a.rs', 'h', 'rust', 0)",
            [],
        ).unwrap();
        let file_id = db.conn().last_insert_rowid();

        db.conn()
            .execute(
                "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, signature)
             VALUES (?1, 'do_work', 'mod::do_work', 'function', 10, 0, 'fn do_work()')",
                [file_id],
            )
            .unwrap();

        let result = investigate(&db, "do_work", &InvestigateOptions::default()).unwrap();
        assert!(result.is_some());

        let r = result.unwrap();
        assert_eq!(r.symbol.name, "do_work");
        assert_eq!(r.symbol.qualified_name, "mod::do_work");
        assert_eq!(r.symbol.signature.as_deref(), Some("fn do_work()"));
        assert!(r.references.is_empty());
        assert!(r.callers.is_empty());
        assert!(r.callees.is_empty());
    }

    #[test]
    fn investigate_uses_the_selected_qualified_identity_for_references() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('lib.rs', 'h', 'rust', 0)",
            [],
        )
        .unwrap();
        let file_id = conn.last_insert_rowid();
        for (name, qname, line) in [
            ("run", "a::run", 1),
            ("run", "b::run", 10),
            ("caller", "main::caller", 20),
        ] {
            conn.execute(
                "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
                 VALUES (?1, ?2, ?3, 'function', ?4, 0)",
                rusqlite::params![file_id, name, qname, line],
            )
            .unwrap();
        }
        let target: i64 = conn
            .query_row(
                "SELECT id FROM symbols WHERE qualified_name='b::run'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let caller: i64 = conn
            .query_row(
                "SELECT id FROM symbols WHERE qualified_name='main::caller'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
             VALUES (?1, ?2, 'calls', 22, 1.0)",
            rusqlite::params![caller, target],
        )
        .unwrap();

        let a = investigate(&db, "a::run", &InvestigateOptions::default())
            .unwrap()
            .unwrap();
        let b = investigate(&db, "b::run", &InvestigateOptions::default())
            .unwrap()
            .unwrap();
        assert!(a.references.is_empty());
        assert!(a.callers.is_empty());
        assert_eq!(b.references.len(), 1);
        assert_eq!(b.callers.len(), 1);
    }

    #[test]
    fn investigate_returns_source_attested_misses_without_calling_them_references() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('lib.rs', 'h', 'rust', 0)",
            [],
        )
        .unwrap();
        let file_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, symbol_key)
             VALUES (?1, 'target', 'module::target', 'function', 1, 0, 'rust:module::target')",
            [file_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'caller', 'module::caller', 'function', 10, 0)",
            [file_id],
        )
        .unwrap();
        let caller = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO ref_resolutions
             (source_id, target_name, kind, source_line, source_col, outcome)
             VALUES (?1, 'target', 'calls', 12, 7, 'unresolved')",
            [caller],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO resolution_census (file_id, content_hash, counts_json) VALUES (?1, 'h', '[]')",
            [file_id],
        )
        .unwrap();

        let result = investigate(&db, "module::target", &InvestigateOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            result.symbol.symbol_id.as_deref(),
            Some("rust:module::target")
        );
        assert!(result.references.is_empty());
        assert_eq!(result.source_attested_unresolved.len(), 1);
        assert_eq!(result.source_attested_unresolved[0].line, 12);
        assert_eq!(
            result.reference_coverage.occurrence_coverage,
            references::OccurrenceCoverage::Complete
        );
    }

    #[test]
    fn investigate_bundles_indexed_source_and_same_file_tests() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/registry.rs', 'h', 'rust', 0)",
            [],
        )
        .unwrap();
        let file_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, end_line, col)
             VALUES (?1, 'all_resolvers', 'registry::all_resolvers', 'function', 10, 12, 0)",
            [file_id],
        )
        .unwrap();
        let target_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, end_line, col)
             VALUES (?1, 'registry_keeps_language_resolvers',
                     'tests::registry_keeps_language_resolvers', 'test', 30, 34, 0)",
            [file_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO code_chunks
             (file_id, symbol_id, content_hash, content, start_line, end_line)
             VALUES (?1, ?2, 'chunk-hash', 'fn all_resolvers() { registry() }', 10, 12)",
            rusqlite::params![file_id, target_id],
        )
        .unwrap();

        let result = investigate(
            &db,
            "registry::all_resolvers",
            &InvestigateOptions::default(),
        )
        .unwrap()
        .unwrap();
        let excerpt = result.source_excerpt.expect("source excerpt");
        assert_eq!(excerpt.file_path, "src/registry.rs");
        assert!(excerpt.content.contains("registry()"));
        assert_eq!(result.nearby_tests.len(), 1);
        assert_eq!(
            result.nearby_tests[0].name,
            "registry_keeps_language_resolvers"
        );
    }
}
